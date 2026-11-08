# Sound in zgx: a design

Status: a draft for review. zgx plays no sound today. This is how it would, in a crate of its own, `zgx-audio`, with a prototype in this branch: a Luau script plays a click on macOS, and a synth's stream plays a tone. Nothing here is merged or promised.

What it covers:

1. Sounds from files, played once or looped, each with a volume and a pan.
2. A low-latency output stream a synth fills, on the audio thread, with no allocation or script there.
3. A Luau API in zgx's style: state-driven, with types in `zgx.d.luau`.
4. The backends on macOS, iOS, Windows and Linux, and how zgx-video's audio track fits.

## What an app pays

Nothing, if it doesn't use sound:

- **Binary.** `zgx-audio` is a crate of its own. zgx, zgx-desktop and zgx-script don't depend on it. A script host gets `ui.sound` from a zgx-script `audio` feature, off by default, the way `scene3d` gives `ui.scene3d`: a build without it holds none of the code.
- **Startup.** `ui.sound` is a lazy module (`crates/zgx-script/src/lazy.rs`): a script pays for it the first time it names it, and the device opens at the first sound played, not when the module loads. In the prototype, a script can require `@audio` and play nothing, and no device or thread exists.
- **Instructions.** Nothing runs in the frame loop. A sound is a command sent when the script asks. The mix runs on the system's audio thread only while something plays. After a few seconds of silence the output stops (`AudioOutputUnitStop`, `IAudioClient::Stop`), so an idle app's audio thread doesn't wake either. "Idle windows do no work" holds.

Whether zgx-run turns `audio` on by default is an open choice (below).

### Measured

zgx-run on the M5 MacBook Pro, built in release with this branch's prototype behind an `audio` feature (`require("@audio")`), off and then on:

| | Off | On |
| --- | --- | --- |
| Binary | 35,605,680 bytes | 35,666,304 bytes (+60,624, +0.17%) |
| `__TEXT` | 29,507,584 | 29,556,736 (+49,152) |
| Images dyld loads at launch | 1,310 | 1,310, the same set |
| First frame, a two-line script, 20 launches each, alternating | median 129.6 ms (125.0–153.5), 686.4M instructions (683.6–696.6) | median 130.1 ms (125.4–154.7), 685.9M instructions (684.4–697.6) |
| Threads, a script that never plays a sound | 7 | 7, none of them audio |
| Threads, the click script | | 12: the system's `com.apple.audio.IOThread.client` and `AudioSession - RootQueue` among them |

Off, zgx-audio isn't in zgx-run's dependency graph at all (`cargo tree`). On:
- **Linking.** The binary links AudioToolbox, but AppKit loads AudioToolbox and CoreAudio in every app anyway, so launch loads nothing new.
- **Startup.** The two sides are within each other's noise.
- **No device until a sound.** A test (`script::tests`) runs a script that doesn't require `@audio`, and one that requires it and plays nothing. Neither tries to open the device. A script that plays a sound does.

## Layers

```
 Luau: ui.sound, ui.synth            zgx-script (feature "audio")
 Rust: Engine  load · play · stream  zgx-audio
       Controller ── commands ──▶ Mixer        (lock-free queues)
                  ◀── garbage ───
 Device: AudioUnit · WASAPI · ALSA/PipeWire · AAudio
```

**The mixer** (`crates/zgx-audio/src/mixer.rs`, in the prototype) runs on the audio thread. It holds up to 64 voices and one stream. It takes commands (play, set, stop, stream) from a single-producer, single-consumer queue of fixed size, and hands back what it lets go of (a sound that ended, a stream replaced) through a second queue. Those are freed on the app's thread. Rendering never allocates, frees, locks, waits or runs a script. `tests/no_alloc.rs` counts the allocator's calls over 5,000 buffers of plays, changes, stops, loops and a stream, and finds none. Make the mixer free an ended sound itself and that test fails.

Other details:

- A change of volume or pan, and a stop, ramp over about 5 ms, so nothing clicks.
- A sound can start at a delay timed to the frame: notes in time, whatever the app's thread is doing when it sends them.
- Pan keeps the near side whole and lowers the far side along a quarter cosine, so a centred sound plays at its recorded level.

**The engine** (`Engine`) opens the device, decodes and keeps sounds (resampled once, at load, to the device's rate, so nothing resamples on the audio thread), and drives the mixer.

**The device** is a module per platform.

## 1. Sounds from files

```rust
let mut audio = zgx_audio::Engine::open()?;
let click = audio.load("sounds/click.wav".as_ref())?;
audio.play(&click, Play { volume: 0.8, pan: -0.5, ..Play::default() });
let rain = audio.play(&audio.load("sounds/rain.wav".as_ref())?, Play { looped: true, ..Play::default() });
audio.set(rain, 0.3, 0.);   // volume, pan, smoothly
audio.stop(rain);           // fades out over 5 ms
```

**Decoding.** The prototype reads WAV itself (zgx-video's reader, moved here) and on macOS converts other formats through `afconvert`. The plan:

- Apple: ExtAudioFile (AudioToolbox, already linked for output). Reads MP3, AAC, ALAC, FLAC, AIFF and CAF at no binary cost.
- Windows: Media Foundation's source reader. MP3, AAC and FLAC.
- Linux and everywhere else: Symphonia (pure Rust: MP3, AAC, FLAC, Vorbis, ALAC), behind a feature.

An open choice is whether every platform uses Symphonia instead, for the same formats everywhere at about a megabyte.

**Long files.** A sound is decoded whole. That's right for UI sounds and samples, wrong for music: three minutes of float stereo at 48 kHz is 69 MB. The plan is to stream sounds over about 10 seconds: a decoder thread fills a ring the voice reads, a second or so ahead. Same `play`, same voice, a different source.

## 2. A stream for a synth

```rust
engine.stream(move |out: &mut [f32], info: Info| {
    // Interleaved stereo, zeroed; voices are added over it afterwards.
    for (i, frame) in out.chunks_exact_mut(2).enumerate() {
        frame.fill(next_sample(info.frame + i as u64));
    }
});
```

The stream is a `Stream` (any `FnMut(&mut [f32], Info) + Send`), moved to the audio thread. It must not allocate, lock or block.

**Talking to it.**

- Parameters such as a cutoff or a volume go through atomics (a `Param`: an `f32` in an `AtomicU32`).
- Events such as note on and note off go through a ring like the mixer's, each stamped with the frame it falls on. The app's thread sends a note a few milliseconds ahead, so a frame that runs late doesn't make the rhythm late.

**Latency.** That's the system's buffer, which the engine asks to shrink:

| System | Default | Asked for |
| --- | --- | --- |
| macOS | 512 frames (10.7 ms) | 128 frames (2.7 ms), the device's buffer frame size |
| iOS | about 23 ms | 5 ms, `AVAudioSession.setPreferredIOBufferDuration` |
| Windows | 10 ms shared | the engine's minimum period, about 3 ms, via `IAudioClient3` |
| Linux (PipeWire) | 1024 frames | a quantum of 128–256 through the node's latency |

On Apple systems the audio thread is real-time by the system. On Windows it joins MMCSS's "Pro Audio" class. On Linux it asks rtkit.

**From Luau.** A script can't run on the audio thread, which is the rule that keeps it glitch-free, so a synth app in Luau plays instruments zgx ships, driven by state:

```lua
local cutoff = ui.use_state(1200)
local synth = ui.synth {
	voices = 8,
	oscillator = "saw",            -- "sine", "square", "triangle", "noise"
	envelope = { attack = 0.005, decay = 0.2, sustain = 0.6, release = 0.4 },
	filter = { kind = "lowpass", cutoff = cutoff, resonance = 0.3 },
	volume = 0.5,
}
-- A key's press and release.
synth:note_on(60, 0.9)              -- MIDI pitch, velocity
synth:note_off(60)
synth:note(64, 0.8, 0.25)           -- on, and off a quarter second later
```

A state given as a parameter (`cutoff`) is written to its atomic when it changes. No script runs per buffer, and none per frame either. An app that needs its own DSP writes it in Rust as a `Stream` and exposes controls through the host's actions, as zgx-video's player does.

How much DSP zgx ships is an open choice. The smallest useful set is oscillators, ADSR envelopes, a biquad filter, a delay and gain/pan per voice.

## 3. The Luau API

```lua
-- Once:
ui.sound.play("click.wav", { volume = 0.8, pan = -0.5 })

-- Following state: plays while `raining` is true, fading in and out over
-- `fade` seconds, its volume following `level`. Adds no node; it stops (with
-- its fade) when the component holding it unmounts.
ui.sound {
	src = "rain.ogg",
	playing = raining,
	loop = true,
	volume = level,
	pan = 0,
	fade = 0.3,
}

-- Read ahead of a first play (decoding a long file takes a moment):
ui.sound.preload { "click.wav", "pop.wav" }

-- Loudness for a meter: a state that changes at the frame rate only while
-- something reads it and something plays.
local level = ui.sound.level()
```

This is zgx's usual shape. A component declares a sound with states for what changes, and Rust re-runs only what reads a state that changed. A `ui.sound` element is a mount-scoped effect, like `video.audio`: it adds no node, and its lifetime is its component's.

Lint rules to add: `ui.sound.play` inside a `ui.computed` or a builder (it plays on every rebuild), and `ui.sound { volume = level:get() }` (read once, never follows).

Proposed types for `crates/zgx-script/types/zgx.d.luau`:

```lua
export type SoundOptions = { volume: number?, pan: number?, delay: number? }
export type SoundProps = {
	src: string,
	playing: Reactive<boolean>?,
	loop: boolean?,
	volume: Reactive<number>?,
	pan: Reactive<number>?,
	fade: number?,
}
export type SynthProps = {
	voices: number?,
	oscillator: ("sine" | "saw" | "square" | "triangle" | "noise")?,
	envelope: { attack: number?, decay: number?, sustain: number?, release: number? }?,
	filter: { kind: ("lowpass" | "highpass" | "bandpass")?, cutoff: Reactive<number>?, resonance: Reactive<number>? }?,
	volume: Reactive<number>?,
}
export type Synth = {
	note_on: (self: Synth, pitch: number, velocity: number?) -> (),
	note_off: (self: Synth, pitch: number) -> (),
	note: (self: Synth, pitch: number, velocity: number?, seconds: number?) -> (),
}
export type SoundModule = typeof(setmetatable({} :: {
	play: (src: string, options: SoundOptions?) -> (),
	preload: (srcs: { string }) -> (),
	level: () -> ReadOnlyState<number>,
}, {} :: { __call: (any, SoundProps) -> Element }))
-- in UiLib: sound: SoundModule, synth: (SynthProps) -> Synth
```

The prototype's bridge is smaller: a package, `require("@audio")`, whose `audio.play` calls the host's `zgx.audio.play` action. The shipping version is the lazy `ui.sound` module above, in zgx-script.

## 4. Backends

| Platform | Output | Prototype | Notes |
| --- | --- | --- | --- |
| macOS | AudioUnit, default output (`auou`/`def `) | yes | Follows the default device as it changes (headphones in or out). |
| iOS | AudioUnit, Remote I/O (`rioc`) | plays in the simulator | Needs the session below. The prototype runs under the default session. |
| Windows | WASAPI, shared, event-driven | no | Through the `windows` crate zgx already uses. Reopen on a device change (`IMMNotificationClient`). |
| Linux | ALSA's API, loaded with `dlopen` | no | Plays on PipeWire, PulseAudio and plain ALSA alike. A binary still starts where there's no libasound. PipeWire's own API later, for lower latency. |
| Android | AAudio, loaded with `dlopen` | no | When zgx runs there. |

**cpal or our own?** cpal covers all of these behind one API, and would be quicker to land. Against it:

- On Linux it links libasound at build and load time: a dev package to build, and a binary that won't start without the library.
- It leaves iOS's audio session to the app.
- It gives less control over buffer size, stopping when idle, and device changes.

Our own backends are small. The macOS and iOS one in the prototype is 250 lines of AudioToolbox calls and adds no dependency. WASAPI and ALSA would be a few hundred lines each. This matches how zgx keeps its dependencies few and its own. The recommendation is our own, with cpal as the fallback if time matters more.

**iOS: the audio session.** An app says what its sound is through `AVAudioSession`'s category, before the output starts:

- `ambient`: the default for UI sounds. It mixes with other apps' audio, and the silent switch silences it.
- `playback`: for a synth or music app. It plays with the silent switch on, and with `UIBackgroundModes: audio` in the app's Info.plist it keeps playing in the background.
- `playAndRecord`: only once there's microphone input (later).

The engine sets the preferred rate (48 kHz) and buffer (5 ms) and activates the session. It handles interruptions (a call, Siri) by pausing and restarting the unit, and a route change (headphones pulled) by pausing playback sounds, as the platform's guidelines ask.

An app chooses its category in its app config (`audio = { category = "playback" }`, read by `app_config.rs`), with `ambient` as the default. macOS, Windows and Linux have no such session: the category is ignored there.

## zgx-video's audio track

zgx-video mixes offline today (`crates/zgx-video/src/audio.rs`):

- It reads WAV itself, and other formats through ffmpeg or `afconvert`.
- It resamples to 48 kHz in straight lines.
- It mixes every `video.audio` clip sample by sample, with fades, loops and per-frame volumes, into one stereo track it muxes into the file.
- `video.audio_data` analyses a sound for bars, levels and waveforms (`analysis.rs`).

With zgx-audio:

- **Decoding moves to zgx-audio** (`zgx_audio::wav`, and the platform decoders as they come), built without a device. zgx-video depends on it with its `device` feature off, so a render on a server still links no audio output. The prototype already holds the same WAV reader and resampler.
- **The offline mix stays in zgx-video.** It's deterministic, tied to frames, and already tested. It could later call the same `Mixer` offline (the mixer's `render` runs without a device, which is how its tests work), so a preview and the file can't drift apart.
- **The player gets sound.** `zgx_video::Player` (the studio's preview, and a video in any app) is silent today. Its composition's track is deterministic, so the player mixes it once and plays it as one voice from the current frame: a seek is a stop and a play at an offset. The picture follows the audio clock while playing, so the two stay in sync.
- **The analysis could serve live sound too.** `ui.sound.level()`, or a spectrum for a visualizer, could use the same FFT as `video.audio_data`, moved to zgx-audio beside the decoders.

## The prototype in this branch

- `crates/zgx-audio`, in the workspace and not published:
  - `ring.rs`: the lock-free queue.
  - `mixer.rs`: voices, stream, ramps, delays and pan.
  - `wav.rs`: WAV reading and resampling.
  - `backend/apple.rs`: AudioUnit output.
  - `lib.rs`: the engine, decoding and caching.
  - `script.rs` and `luau/audio.luau`: `require("@audio")` for a script host, behind the `script` feature.
- `examples/click.rs`: headless, with no window. A script (`examples/click/app.luau`) plays a click three times, left, middle and right, a fifth of a second apart, timed on the audio thread. Then a stream plays a 440 Hz tone with an envelope. On an M5 MacBook Pro, `cargo run -p zgx-audio --features script --example click` printed `script: 3 clicks sounded, peak 0.78, 33792 frames at 48000 Hz` and `stream: peak 0.19, 24064 frames`.
- Tests: the mixer's behaviour offline (levels, pan, a delay across buffers, a loop and a stop's fade, the stream under the voices, freeing off the audio thread), the queue across threads, WAV reading and resampling, and `no_alloc`.

- The same example in the iOS simulator (built for `aarch64-apple-ios-sim`, run with `xcrun simctl spawn`) printed `script: 3 clicks sounded, peak 0.78, 33280 frames at 48000 Hz` and `stream: peak 0.19, 23552 frames`.
- zgx-run's `audio` feature, off by default, installs `@audio` for an app's script. It exists for the measurements above. zgx-run can't be published with it until zgx-audio is.

Not yet: buffer-size requests, device changes, the idle stop, decoders beyond WAV and `afconvert`, iOS's session, and Windows and Linux. Nothing in zgx-script changes yet.

## Phases

1. **zgx-audio on Apple systems.** Harden the prototype: buffer size, device changes, idle stop, ExtAudioFile, streaming long files.
2. **`ui.sound` in zgx-script** behind `audio`: the lazy module, types, lint rules, `docs/reference/sound.md`, tests through the headless harness with an offline mixer.
3. **Windows and Linux.** WASAPI, ALSA through `dlopen`. A Linux probe in `scripts/linux_smokes.py` plays into a PipeWire null sink, records it and compares it with the expected mix. A Windows run does the same through a loopback capture.
4. **iOS.** The session, interruptions and route changes, checked on a phone.
5. **`ui.synth`.** Its instruments, note events timed to the frame, and later MIDI input (CoreMIDI, Windows MIDI, ALSA sequencer).
6. **zgx-video.** Shared decoding, and the player's sound.

## Open choices

1. **Backends:** our own (recommended: no new dependencies, full control) or cpal (quicker, but links libasound on Linux and leaves iOS's session to us anyway).
2. **zgx-run's default:** turn `audio` on for every app it runs (scripts just work, and the cost is the code's size in the binary, still with no startup or idle cost), or make apps ask for it.
3. **Decoders:** each platform's own (no size, formats differ a little by OS) or Symphonia everywhere (the same formats everywhere, about a megabyte).
4. **How much synth zgx ships:** the small set above, more (reverb, LFOs, modulation routing), or none (synths in Rust only).
5. **iOS's default category:** `ambient` (polite, silenced by the switch) or `playback`.
6. **Where the mix's rate comes from:** the device's (the prototype: nothing resamples while playing) or always 48 kHz (simpler, with a resampler on the audio thread when a device runs at 44.1 kHz).
