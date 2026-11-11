# zgx-audio: playback, mixing and spatial sound (design, Nov 10 2026)

Read on origin/main `e89ada127`. Nothing here is built yet. The questions it raised are settled in [section 12](#12-decisions). The rendering side is [design-3d-aaa.md](design-3d-aaa.md).

## 1. Where zgx is today

- **zgx plays no sound.** No crate opens an output device; there's no dependency on any audio library.
- **zgx-video mixes offline only:** `crates/zgx-video/src/audio.rs` holds 48 kHz interleaved stereo `f32`, reads WAV (other formats through ffmpeg or afconvert), and mixes into a file that ffmpeg muxes. It never reaches a device.
- **There's no iOS host on main,** so nothing about iOS audio (sessions, interruptions, the silent switch) can be verified yet.

## 2. Principles

1. **An app that doesn't use audio pays nothing:** a new crate, `zgx-audio`, that no core crate depends on. zgx-desktop, zgx-script and zgx-3d reach it only behind an `audio` Cargo feature, off by default everywhere until the maintainer says otherwise.
2. **Silent means asleep.** With nothing playing, the device stream is stopped, the decode thread is parked, and the process's wakeups are those of an app without audio. This is measured, not assumed.
3. **The audio callback never waits:** no locks, no allocation, no file or system calls, no Luau. Everything it needs arrives through fixed-size queues.
4. **Sample-accurate and deterministic.** Every start, stop and parameter change has a time on the device's sample clock. The same mixer runs offline against a virtual clock, which is how tests compare output exactly and how zgx-video can render a soundtrack.
5. **The system is in charge** of the output device, the volume, interruptions and the user's sound settings. zgx-audio follows them and changes none.
6. **Primitives only.** Footsteps, music systems, dialogue and ambience managers are the app's.

## 3. Crates and features

| Where | What | Depends on |
| --- | --- | --- |
| `crates/zgx-audio` | Devices, the mixer, voices, buses, effects, decoding, spatial maths, the offline renderer | Nothing of zgx's. Platform audio APIs; decoders behind features (`vorbis`, `opus`, `mp3`, `aac`, `hrtf`). |
| `zgx-desktop`, feature `audio` | Starts and stops the engine with the app, follows the default device, reads the user's sound settings, pauses on suspend | zgx-audio |
| `zgx-3d`, feature `audio` | Emitters on objects, the listener on the camera, occlusion rays, reverb zones | zgx-audio |
| `zgx-script`, feature `audio` | The Luau `audio` library, `ui.sound3d`, `ui.reverb_zone`, types, lint | zgx-audio (and zgx-3d's glue with `scene3d`) |

A separate crate, not a zgx-desktop feature: Rust apps that draw nothing (a tool, a test) can use it, the offline renderer needs no window, and a feature of zgx-desktop couldn't be reached from zgx-3d without tying 3D to the desktop host.

## 4. Threads

```
 main thread            control queue (SPSC, fixed)         audio callback (the OS's thread)
 ───────────  ───────────────────────────────────────────▶  ─────────────────────────────────
  handles: Sound, Voice, Bus, Emitter                          drain commands due this block
  commands stamped with a sample time                          voices → buses → effects → master → limiter
                                                               write the device's buffer
              ◀───────────────────────────────────────────
               return queue: finished voices, meters, freed buffers, underrun counts

 decode thread (one, parked when no stream is open)
  fills each stream's ring buffer about 0.5 s ahead; decodes whole sounds when asked
```

- **Main thread:** the API. A call makes a command (play, stop, set a parameter, at a sample time) and pushes it to a fixed-size single-producer queue. If the queue is full the call returns an error; it never blocks.
- **Audio callback:** owned by the platform. Each block it applies the commands due, at their exact frame within the block, mixes, and writes. It touches only memory made before it started: a fixed pool of voices (64 real by default), buses and effect state. Buffers it's done with go back on the return queue to be freed on the main thread, since `free` isn't allowed here.
- **Decode thread:** one thread, asleep on a condition variable unless a stream needs filling or a load was asked for. A stream that runs dry fades out over 5 ms, counts an underrun, and fades back when data arrives.
- **Voices past the limit:** each voice has a priority; when the pool is full the quietest of the lowest priority is stolen with a 5 ms fade. Looping voices that are stolen or inaudible become *virtual* (their position advances, nothing mixes) and return when there's room.
- **Starting and stopping the device:** the stream starts with the first sound and stops after a hold with nothing audible (1 s by default: reverb tails and fades count as audible). `Audio::prewarm()` starts it early for an app that knows a sound is coming. See [decision 4](#12-decisions): a cold start may be slower than the platform's own click.
- **Device changes:** the engine listens for the system's default-device and format notifications and rebuilds its stream on the new one, keeping voices and their positions. Sounds are held at their file's rate and resampled per voice, so a rate change needs no reload.

## 5. Playback

```rust
let audio = Audio::new()?;                         // no device opened yet
let click = audio.load(Source::bytes(include_bytes!("click.wav")))?;   // decoded whole, off the main thread
let voice = audio.play(&click, Play::new().bus(&ui).gain(0.8));
audio.play(&tick, Play::new().at(audio.now() + audio.seconds(0.250))); // sample-accurate
let music = audio.stream(Source::file("score.ogg"), Stream::new().looping(true).fade_in(2.0))?;
music.crossfade_to(&next, 3.0);
voice.stop_after(0.05);                            // a fade, never a cut
```

- **Sounds** are decoded whole and shared (`Arc`); **streams** decode as they play and hold about half a second.
- **Clock:** `audio.now()` is the device's sample clock, running only while the stream runs; `at(time)` places a start on an exact frame. Loop points are sample positions.
- **Every start and stop is ramped** (1–5 ms) unless the sound begins at a zero crossing, so nothing clicks that wasn't meant to.
- **UI sounds** are ordinary sounds on a `ui` bus, with one difference: that bus follows the user's interface-sound setting ([section 9](#9-the-system)).

## 6. Mixer

- **Buses** form a tree under `master`: gain, mute, solo, a pan and an effect chain. Up to 64 with master; a bus's two block buffers and its effect slots are made when it's added, on the main thread, so a bus never added costs none, and the mixer's loops run over the buses added. Buses can't be removed.
- **Sends:** a voice sends to up to two buses besides its own, each at a smoothed level (a reverb zone's room, a binaural bus, an app's aux). Bus-to-bus sends are open (section 12). `ui`, `music` and `world` exist by default; apps add their own.
- **Parameters** are set with a ramp time and smoothed per sample, so a slider never zips. They're atomics underneath: a UI state or a `ui.tween` binding drives one with no script per frame.
- **Ducking:** `music.duck(&dialogue, Duck { by_db: -9., attack: 0.05, release: 0.4 })`, a side-chained gain driven by another bus's level.
- **Stock effects:** low-pass and high-pass (state-variable, 12 or 24 dB), a parametric EQ band, reverb (a feedback delay network: size, decay, damping, pre-delay, wet), delay, compressor, and a look-ahead limiter that sits on `master` by default so nothing clips.
- **Fades and crossfades** are gain ramps with a curve (linear, equal-power, exponential).

### The custom effect interface

```rust
pub trait Effect: Send + 'static {
    /// Called off the audio thread before first use, and again when the format changes.
    fn prepare(&mut self, format: Format, max_block: usize);
    /// Called on the audio thread. Interleaved frames in place. No locks, allocation or system calls.
    fn process(&mut self, block: &mut [f32], context: &Context<'_>);
    /// How long output continues after input stops (a reverb's tail), so the engine knows when silence is real.
    fn tail(&self) -> Duration { Duration::ZERO }
}
let handle = bus.add_effect(MyEffect::new());
handle.set(MY_CUTOFF, 800.0, Ramp::ms(20));        // parameters by index, smoothed by the engine
```

**Promise:** the trait above, `Context` (sample time, format, the parameter values for this block, a side-chain input if one was asked for), and that `process` is called with at most `max_block` frames. In 0.x that means: not broken in a patch release, and in a minor release only with a changelog entry. A debug build checks the rules: an allocation or a lock taken inside `process` panics with the effect's name.

**Luau can't write effects.** A script can't run on the audio thread. Scripts get the stock effects with every parameter bindable; custom DSP is Rust, or a native plugin.

## 7. Spatial audio

```rust
let hum = world.add_emitter3d(crane_cabin, Emitter3d::new(&engine_loop)
    .looping(true).gain(0.7)
    .falloff(Falloff::inverse(2.0, 60.0))          // reference distance, maximum
    .cone(60., 140., 0.3)
    .doppler(1.0).occlusion(true));
world.set_listener(Listener::Camera);              // the default with the feature on
world.add_reverb_zone(ReverbZone::box_(min, max).preset(Reverb::WAREHOUSE).fade(4.0));
```

- **Emitters** follow their object's world transform; the **listener** follows the camera unless set. Positions are read when the world changes, sent as commands, and interpolated over the audio block, so a fast pass-by doesn't step. A still world sends nothing.
- **Distance:** inverse, linear or exponential roll-off with reference and maximum distances, or the app's own curve; air absorption as a distance-driven low-pass.
- **Doppler** from the emitter's and listener's velocities along the line between them, as a pitch change in the voice's resampler, smoothed and clamped.
- **Panning:** equal-power for stereo speakers; vector-based amplitude panning for surround layouts; **HRTF** for headphones (convolved per emitter, behind the `hrtf` feature: by default through a spherical-head model made in code, Woodworth's interaural delay and Brown and Duda's head shadow; an app supplies its own measured set to replace it). In a world, `Emitter3d::binaural(true)` plays an emitter centred into one of the world's heads instead of panning it. A head is a bus with a spherical-head binaural on it, facing the emitter each frame it moves. A world makes up to eight as needed and keeps them for its life. A head let go is given to the next binaural emitter, and past eight an emitter is panned. A binaural emitter's sends still feed the zones' rooms. The platform's own spatialisers aren't used: they differ per OS and can't be rendered offline or compared in tests. [Decision 6](#12-decisions).
- **Occlusion and obstruction:** rays from the listener to each audible emitter against the world's meshes (the same triangles picking uses), on the main thread, a budget of rays per frame spread round-robin, only when either end moved. A mesh of more than 256 triangles is met through a tree over its triangles, made once a geometry on a thread of its own (320,000 triangles: about 50 ms, 5.5 MB), and tested triangle by triangle until it's made. A ray stops at the first triangle it meets. The result is a gain and a low-pass cutoff, smoothed. Apps can mark meshes as not blocking sound, give a mesh a transmission value, or supply the query themselves. Diffraction and propagation paths aren't modelled.
- **Reverb zones** are volumes the app places, each with reverb settings. The listener's position picks the zones it's in, weighted by a fade distance, and emitters send to that reverb; an emitter in a different zone from the listener sends less. One reverb instance per active zone, two at most by default.
- **Budget rule:** emitters past the voice limit or below an audibility threshold are virtual and cost a distance check each.

## 8. Formats

| Format | How | Feature |
| --- | --- | --- |
| WAV (PCM 8/16/24/32, float) | zgx-audio's own reader | always |
| Ogg Vorbis | A pure-Rust decoder | `vorbis` |
| Opus (in Ogg) | libopus (BSD), or a Rust decoder if one is complete | `opus` |
| MP3 | A Rust decoder; its patents have expired | `mp3` |
| AAC | The platform's decoder only: AudioToolbox on Apple systems, Media Foundation on Windows. Nothing on Linux. | `aac` |

All decoding is off the main thread. Each decoder's binary size is measured before its feature becomes anyone's default, and each dependency's licence is listed for the maintainer first ([decision 3](#12-decisions)).

## 9. The system

- **Platforms:** CoreAudio on macOS (an output unit on the default device) and iOS (with an audio session); WASAPI shared mode on Windows; PipeWire on Linux, PulseAudio where PipeWire isn't there, ALSA last. Shared mode everywhere: zgx never takes a device exclusively.
- **Backends:** proposal: start on `cpal` behind zgx-audio's own small `Device` trait, and measure it against each requirement (wakeups when stopped, start latency, buffer sizes, device-change handling, native PipeWire, size). Where it fails one, write that platform's backend directly behind the same trait, as the project already maintains its own forks where upstream doesn't fit. [Decision 2](#12-decisions).
- **Device and volume:** the system's default output and its changes are followed; the system volume applies to zgx's output as to any app's. zgx-audio has no API to change either.
- **Interruptions (iOS):** the session's interruption notifications pause the engine and resume it when the system says to. The session category defaults to *ambient* (mixes with other apps, obeys the silent switch); an app that is a player asks for *playback*. Built and verified only once an iOS host is on main.
- **Suspend and hidden windows:** the engine pauses when the app is suspended. A hidden or minimised window keeps playing unless the app asks otherwise (music players exist).
- **The user's sound settings:** the `ui` bus is silent when the user has turned interface sounds off (macOS "Play user interface sound effects", GNOME's `event-sounds`; Windows has no single switch, so its sound scheme "No Sounds" is read). Other buses aren't affected: a game's sound isn't an interface sound. Apps read `audio.ui_sounds_enabled()` if they need to know.
- **Latency:** the stream asks for the device's low-latency shared buffer (256–512 frames at 48 kHz, 5–11 ms) and reports what it got in `audio.stats()`.

## 10. Luau API

```luau
local click = audio.sound("assets/click.wav")           -- loaded once, off the main thread
local volume = ui.state("music_volume", 0.8)

audio.bus("music"):gain(volume)                          -- a binding: the slider drives it with no script
local score = audio.music("assets/dawn.ogg", { loop = true, fade_in = 2 })

ui.button { sound = click, "Save" }                      -- plays on activation, on the ui bus
ui.button { on_click = function() audio.play(click, { gain = 0.5, at = audio.now() + 0.1 }) end, "Later" }

ui.scene3d {
	ui.model { src = "crane.glb",
		ui.sound3d { src = "assets/engine.ogg", loop = true, gain = engine_gain,
		             falloff = { ref = 2, max = 60 }, doppler = 1, occlusion = true } },
	ui.reverb_zone { shape = "box", size = { 30, 12, 60 }, preset = "warehouse", fade = 4 },
}
```

- **Library:** `audio.sound`, `audio.music`, `audio.play`, `audio.bus` (gain, mute, solo, effects, duck), `audio.now`, `audio.prewarm`, `audio.stats`, `audio.ui_sounds_enabled`. Voice handles: `stop`, `fade`, `gain`, `pitch`, `playing` (a readable state).
- **Elements:** a `sound` prop on activatable controls; `ui.sound3d` (a child of a 3D object or of the scene) and `ui.reverb_zone` inside `ui.scene3d`. Every numeric prop takes a number, a state, a function or a binding.
- **Types** in `types/zgx.d.luau.in`; reference pages `docs/reference/audio.md` and an audio section in `docs/reference/3d.md`.
- **Lint rules** (new):
  - `audio.sound` or `audio.music` called inside a component or row body (it loads on every build: load once at the top);
  - `audio.play` from `ui.every` under 50 ms or from a per-frame callback (schedule with `at` instead);
  - a file over about 10 s loaded with `audio.sound` (it's decoded whole: use `audio.music`);
  - a gain or pitch written from script in a loop (bind a state);
  - `ui.sound3d` with `loop` and no `falloff.max` (it never becomes virtual).
- **Plugins:** a sandboxed plugin's sounds count against its memory; it plays only on a bus the host gives it, with a voice limit of its own.

## 11. Budgets, tests and milestones

### Budgets (targets, to be measured)

| | Target |
| --- | --- |
| Wakeups with nothing playing | The same as the app without the `audio` feature, over 60 s |
| Memory at rest, engine made, nothing loaded | Under 1 MB, and no device stream open |
| Click to sound, stream running | One device buffer plus the output latency the device reports (typically under 15 ms) |
| Click to sound, stream stopped | Measured against the platform's own click (NSSound, `PlaySound`); if slower, [decision 4](#12-decisions) |
| Callback time, 64 voices, 3 buses with effects, limiter | Under 10% of the block period at the 99.9th percentile, M5 and a mid-range x86 |
| The same with 16 HRTF emitters and 2 reverbs | Under 25% |
| Underruns in a 10-minute soak under UI load (heavy scroll, a 3D scene animating) | 0 |
| Added to `zgx-run`'s `dist` binary, core with WAV | Under 300 KiB; each decoder measured separately |

### Tests

- **Offline renders against references:** the mixer rendered through the virtual clock, compared with checked-in PCM at a tolerance of −90 dB (floating point differs across architectures by less). Covers scheduling to the sample, ramps, each effect's response (impulse and sweep), ducking, voice stealing, loop points, resampling quality (a sweep's aliasing below a stated level), panning laws, roll-off curves, Doppler pitch, and HRTF against its source responses.
- **Real-time rules:** a test binary of its own (a counting global allocator) that fails if the callback allocates, frees or blocks, run over every stock effect and the voice paths.
- **Devices:** on Linux CI, a null sink plus a monitor capture: the probe plays a known signal and checks the captured samples and their timing, registered in `scripts/linux_smokes.py`. On macOS and Windows the engine's own counters (callbacks, frames written, underruns, the time each callback took) prove the stream ran.
- **Idle:** wakeups over 60 s with the feature on and nothing playing, against the same app without it; then play one click and check the stream stops after the hold and wakeups return to the baseline.
- **Machine rule:** automated runs on the maintainer's Macs open the stream with master gain at zero: the callback runs and counters count, nothing is heard. Anything audible, and anything needing a different output device, is asked for first. No test changes a system setting.
- **Gate cells:** `audio-mix` (64 voices, one second, offline), `audio-ui` (100 scheduled clicks), `audio-spatial` (16 HRTF emitters, moving), each rendered offline so cachegrind counts are repeatable. The UI cells stay within +0.02%; binaries without the feature must not change in size.
- **Fail first:** each PR names the test that failed before it.

### Milestones

| | Milestone | Exit criteria |
| --- | --- | --- |
| A1 | **Core, UI sounds first:** the engine, device backends on macOS, Windows and Linux, the callback and queues, voices, sample-accurate scheduling, buses with gain, mute, solo and fades, the limiter, WAV and Vorbis, streams, the offline renderer, the `ui` bus and the user's setting, `Audio` in Rust and `audio.*` in Luau with types, lint and docs, the gate cells. | Offline references match; no allocation in the callback; idle wakeups equal the no-audio app's; click latency measured on all three systems and compared with the platform's; 0 underruns in the soak; UI gate cells flat; sizes unchanged without the feature. |
| A2 | **Spatial:** emitters and the listener in zgx-3d, roll-off, cones, Doppler, panning, HRTF, occlusion rays, reverb zones, `ui.sound3d` and `ui.reverb_zone`. After R3 of the 3D plan, when lights and shadows have settled `world.rs`. | Offline spatial references match; occlusion costs within its ray budget; a still world sends no commands; the demo's harbor sounds are placed with it. |
| A3 | **Mixer and formats complete:** EQ, reverb, delay, compressor, ducking, the `Effect` trait with its debug checks, Opus, MP3, AAC, stream memory budgets, the iOS session if a host exists, zgx-video rendering a soundtrack through the offline mixer. | Each effect's response within tolerance; stability promise documented; licences approved. |

### The demo

| In the harbor | Built in the demo from | Primitives |
| --- | --- | --- |
| Rain on steel, on water, on the cabin roof | Looping layers chosen by what the listener is under, cross-faded | Streams, buses, crossfades, reverb zones |
| Crane motors, a ship's generator, a buoy bell | Emitters on the models | `ui.sound3d`, roll-off, cones, Doppler, occlusion by container stacks |
| Gulls, distant horn | One-shots scheduled at random intervals from Rust | Scheduling, HRTF, air absorption |
| Under the gantry, inside the cabin | Zones | Reverb zones with fades, occlusion low-pass |
| Wind following the scene's wind uniform | A filtered noise loop whose cutoff and gain bind to the same state the water's shader reads | Bindable effect parameters |
| HUD clicks | `sound =` on controls | The `ui` bus, the user's setting |

All of the demo's sounds are synthesised or recorded for it.

## 12. Decisions

Settled Nov 10 2026 by the coordinator, the maintainer having said designs need no approval. Each can be reopened by a measurement.

1. **A crate of its own, `zgx-audio`,** with `audio` features in zgx-desktop, zgx-3d and zgx-script, all off by default.
2. **Device backends start on `cpal` behind our own `Device` trait.** A1 measures it per platform against section 9's requirements; a platform where it fails one gets a direct backend behind the same trait.
3. **One small decoder per format.** Dependencies under MIT, Apache-2.0, BSD or zlib are added on a size measurement; anything else (MPL-2.0 included) goes to the maintainer first.
4. **Idle wins:** the stream stops after a 1 s hold, with `prewarm()`. A1 measures the cold start against the platform's own click; if it loses audibly, UI sounds on that platform go through the platform's click service.
5. **The `ui` bus obeys the user's interface-sound setting; other buses don't.**
6. **HRTF is our own convolution**, its built-in default an analytic spherical-head model made in code. No measured dataset ships: nothing that requires attribution goes into the repository, so an app supplies its own measured set through `HrtfSet::new`.
7. **No DSP in Luau.**
8. **Automated runs on the maintainer's Macs are at zero gain;** anything audible is asked for first.
9. **iOS** waits for the host in draft to land.

### Open

- **Bus-to-bus sends.** The mixer sums buses in one pass, children before parents: a bus is added after its parent, so its index is higher, and the pass runs from the highest index down. A bus's chain runs once, when its turn comes, on whatever reached it by then. The ordering rule for a bus-to-bus send: **its target must come after its source in that pass** (a lower index), so the source's block is complete when it's added in, and the target's chain hears it in the same block. A send against the order (to a bus later added, or to a descendant) would either be heard a block late, as one-block feedback, or need a second pass. Either is refused until a use needs it. Cycles can't occur under the rule, since the order is total. A send to an ancestor is allowed by the rule and is heard twice, once through the tree and once by the send, as an aux to `master` would be. Not built now: per-voice sends cover zones, binaural and an app's aux.

- **Occlusion trees per world.** A large mesh's tree is made and held per world: one geometry blocking in two worlds is built and held twice. A process-wide cache keyed by geometry id, holding weak references, would share it. Not built: no scene so far has the same large mesh in two worlds.

**The bar,** as for rendering: the mixer is measured against established engines' published figures and an open one run on the same machine (callback time per voice, latency, idle wakeups), and merges when it ties or beats them or says why not.
