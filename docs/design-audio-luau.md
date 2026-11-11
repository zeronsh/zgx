# zgx-audio in Luau and the desktop host (design addendum, Nov 10 2026)

The spec for A1's last step in [design-audio.md](design-audio.md) (section 10): zgx-desktop's and zgx-script's `audio` features, the Luau `audio` library, `sound =` on controls, types and lint. The user's page is [reference/audio.md](reference/audio.md). Everything below calls zgx-audio's public Rust API. Built in zgx-desktop, zgx-script and zgx-run as section 9 says, where it departs from what's above.

## 1. Features and ownership

| crate | feature | adds |
| --- | --- | --- |
| zgx-audio | `device` | `Audio::new`, the cpal backend |
| zgx-audio | `vorbis` | Ogg Vorbis for `load` and `stream` |
| zgx-desktop | `audio = ["dep:zgx-audio", "zgx-audio/device"]` | the app's engine, the ui-sound setting, voice-ended wakes |
| zgx-script | `audio = ["dep:zgx-audio"]` | the `audio` global, `sound =`, types, lint |
| zgx-run | turns on both, with `zgx-audio/vorbis` | |

All are off by default; no core crate depends on zgx-audio. zgx-script's `audio` doesn't turn on the device: a headless host (tests, `zgx test`, zgx-video) gives it an offline engine.

**One engine per application, made on first use.** `zgx_audio::Audio::new(Config::default())` is cheap and touches nothing of the system (tested: no device call, not even the format query, until the first play). The desktop host holds `OnceCell<Audio>` and hands clones to each `ScriptHost` that asks (`ScriptHost::set_audio(audio)`). A script that never reads `audio` makes none. One that only loads sounds and sets gains still opens nothing: commands that make no sound are applied without a device.

## 2. The desktop host's part (zgx-desktop, `audio`)

1. **Voices ending.** `audio.on_voices_ended(move || proxy.send_event(UserEvent::AudioVoicesEnded))`. It is called from zgx-audio's host thread (never the audio thread) at most every 100 ms while the device runs. On that event the app calls `audio.take_ended()` and gives the ids to each script host (`ScriptHost::voices_ended(&ids)`), which sets the matching `playing` states false. No polling: while nothing plays, nothing is called.
2. **The user's interface-sound setting.** It's read at startup and on change, and passed to `audio.set_ui_sounds(enabled)` (no device touched). The implementing lane must find each platform's source and change notification and verify them, reading only, never writing the setting:
   - macOS: the global domain's `com.apple.sound.uiaudio.enabled` (absent means on); watch for changes (a distributed notification, or `NSUserDefaults` KVO).
   - Linux: GSettings `org.gnome.desktop.sound` `event-sounds` (GNOME); a desktop without it counts as on.
   - Windows: the current sound scheme, `HKCU\AppEvents\Schemes` default `.None` means off.
3. **Suspend.** zgx-desktop has no suspended state on macOS, Windows or Linux, so there's nothing to do yet; iOS waits for its host (design decision 9).
4. **Shutdown.** Dropping the last `Audio` joins its threads and closes the stream on the host thread.

## 3. The `audio` global (zgx-script, `audio`)

Each row is one call and what it does in Rust. `audio` is the host's `zgx_audio::Audio`. "Followed" means zgx-script's usual effect for a reactive prop: it reruns when what it reads changes, and only then.

| Luau | Rust | per frame? |
| --- | --- | --- |
| `audio.sound(src, opts)` | a worker thread runs `audio.load(Source::file(path))`; the handle holds `Option<Sound>` plus `ready` and `error` states set when it returns | — |
| `audio.play(sound, opts)` | `audio.play(&sound, Play::new().bus(&b).gain(g).pan(p).pitch(r).at(Time(t)).looping(l).loop_frames(a..b).priority(n).fade_in(ramp))`; `at` seconds become `Time((s * audio.rate() as f64) as u64)`; loop points become frames at `sound.rate()` | — |
| a play before its sound is ready | kept by zgx-script with the time it was asked; played when the load returns if within 250 ms, else dropped and counted as `late` | — |
| `audio.music(src, opts)` | `audio.stream(Source::file(path), play)` on `music` by default | — |
| `gain`/`pan`/`pitch` as a state or function | followed: each change is `voice.set_gain(v, Ramp::ms(10.))` (and `set_pan`, `set_pitch`) | no |
| `gain`/`pan`/`pitch` as a binding (`ui.tween`, `ui.spring`) | the animation engine's tick writes `voice.set_gain(v, Ramp::seconds(frame_interval))`: one command a frame while it eases, none at rest | in Rust, no script |
| `voice:stop(s)` | `voice.stop_after(Ramp::seconds(s))`, or `voice.stop()` | — |
| `voice:fade(g, s, curve)` | `voice.set_gain(g, Ramp::seconds(s).curve(c))` | — |
| `voice:at(t)` | `voice.at(Time(..))` | — |
| `voice.playing` | a state made true at `play`, made false by `ScriptHost::voices_ended` for `voice.id()`; offline hosts call `audio.take_ended()` after each render | — |
| `audio.bus(name)` | `audio.bus(name)` (nil if there's none) | — |
| `audio.add_bus(name, { parent })` | `audio.add_bus(name, &parent)` | — |
| `bus:gain(value)`, any reactive kind | `let drive = bus.drive()`; a literal, state or function is followed and writes `drive.set(v)`; a binding's tick writes `drive.set(v)` each frame. The mixer reads the drive each block and glides over 10 ms: no command at all | no, even for a binding |
| `bus:fade(g, s, curve)` | `bus.set_gain(g, Ramp::seconds(s).curve(c))` (it takes over until the drive is set again) | — |
| `bus:mute(v)`, `bus:solo(v)` | followed: `bus.mute(v)`, `bus.solo(v)` | no |
| `bus:at(t)` | `bus.at(Time(..))` | — |
| `audio.now()` | `audio.now().0 as f64 / audio.rate() as f64` | — |
| `audio.prewarm()` | `audio.prewarm()` | — |
| `audio.stats()` | `audio.stats()` as a table, plus zgx-script's own `late` count | — |
| `audio.ui_sounds_enabled` | a readable state set from the host's setting (the same value it passes to `set_ui_sounds`) | — |

Curves: `"linear"` → `Curve::Linear`, `"equal_power"` → `Curve::EqualPower`, `"exponential"` → `Curve::Exponential`.

**Errors.** `Error::QueueFull` (more than 1024 commands waiting: a script sending in a loop) is a script error naming the call. A load failure is the sound's `error` state, not a thrown error.

**Hot reload.** Sounds are cached by path and modified time across reloads (as images are). Voices of a reloaded script are stopped with their 5 ms fade, and streams are stopped, so a reload doesn't leave music playing twice.

## 4. `sound =` on controls

A prop read by every activatable element: `ui.button`, `ui.toggle`, `ui.checkbox`, menu items, segmented options, and rows with `on_click`. In `lower.rs`, where activation dispatches `on_click`, zgx-script calls `audio.play(&sound, Play::new().bus(&ui))` first, then the handler. The value is a `Sound` handle, `{ sound, gain, pitch }`, a state of either, `false` or nil. Activation by keyboard and by assistive technology plays it too. It's in zgx-components' controls through their underlying `ui.button`, with no change there.

## 5. Types (to add to `types/zgx.d.luau.in`; that file is shared, so the lane asks before editing)

```luau
export type Curve = "linear" | "equal_power" | "exponential"
export type Fade = number | { seconds: number, curve: Curve? }

export type Sound = {
	read ready: Readable<boolean>,
	read error: Readable<string?>,
	read duration: number,
}

export type SoundOptions = { preload: boolean? }

export type PlayOptions = {
	bus: (string | Bus)?,
	gain: Reactive<number>?,
	pan: Reactive<number>?,
	pitch: Reactive<number>?,
	at: number?,
	loop: boolean?,
	loop_start: number?,
	loop_end: number?,
	priority: number?,
	fade_in: Fade?,
}

export type Voice = {
	read playing: Readable<boolean>,
	stop: (self: Voice, seconds: number?) -> (),
	fade: (self: Voice, gain: number, seconds: number, curve: Curve?) -> (),
	gain: (self: Voice, value: Reactive<number>) -> (),
	pan: (self: Voice, value: Reactive<number>) -> (),
	pitch: (self: Voice, value: Reactive<number>) -> (),
	at: (self: Voice, time: number) -> Voice,
}

export type Bus = {
	gain: (self: Bus, value: Reactive<number>) -> (),
	fade: (self: Bus, gain: number, seconds: number, curve: Curve?) -> (),
	mute: (self: Bus, value: Reactive<boolean>) -> (),
	solo: (self: Bus, value: Reactive<boolean>) -> (),
	at: (self: Bus, time: number) -> Bus,
}

export type AudioStats = {
	voices: number, stolen: number, dropped: number, late: number, dry: number,
	streams: number, underruns: number, running: boolean, starts: number,
	stops: number, peak: number,
}

export type SoundProp = Sound | { Sound, gain: number?, pitch: number? } | Readable<Sound?> | false

declare audio: {
	sound: (src: string, opts: SoundOptions?) -> Sound,
	music: (src: string, opts: PlayOptions?) -> Voice,
	play: (sound: Sound, opts: PlayOptions?) -> Voice,
	bus: (name: string) -> Bus?,
	add_bus: (name: string, opts: { parent: string? }?) -> Bus,
	now: () -> number,
	prewarm: () -> (),
	stats: () -> AudioStats,
	ui_sounds_enabled: Readable<boolean>,
}
```

`sound: SoundProp?` joins the props of the activatable elements. The `.in` file's generator gates `declare audio` on the feature, as it does `scene3d`.

## 6. Lint (`lint.rs`, entries in `RULES`)

| rule | finds |
| --- | --- |
| `audio-load-in-builder` | a call to `audio.sound` or `audio.music` inside a component's function or a `ui.list`/`ui.each` row function: the same scope test `state-in-builder` uses |
| `audio-play-timer` | `audio.play` inside a `ui.every(ms, …)` with a literal `ms < 50`, or inside an `on_frame`/`anime` `onUpdate` callback |
| `audio-sound-too-long` | `audio.sound("literal")` whose file, beside the script, lasts over 10 s. The rule reads only the file's header (WAV `fmt` and `data` sizes, Ogg's last granule position), never decodes. A missing file is silence: `zgx check` reports paths |
| `audio-param-in-loop` | `:gain(`, `:pan(` or `:pitch(` with a numeric (not reactive) argument inside `ui.every` or a per-frame callback |
| `sound3d-no-max` | A2: `ui.sound3d` with `loop = true` and no `falloff.max` |

Each finding says the cheaper way, in the style of the table in AGENTS.md (which gets five rows for audio).

## 7. Tests the zgx-script lane writes

With an offline engine (`Audio::offline(Config::default())`, its `Offline` held by the test host, rendered a block at a time):

- `sound =` plays on activation by click, Enter and accessibility action, on `ui`, before `on_click`. The rendered output's first non-zero frame is the block of activation.
- `bus:gain(state)` sends no command per block: `Stats` shows the same command count after 100 blocks with the state unchanged. A `ui.tween` on it produces the glide in the rendered output with no script call per frame, counted with the existing per-frame script counters.
- `voice.playing` turns false after the voice ends, with `take_ended` called by the host's render loop.
- A play before its sound is ready plays when ready within 250 ms, and is dropped (and counted) after.
- Hot reload stops the old script's voices and streams with their fades.
- A plugin's sounds go only to its bus, and a ninth voice is refused.
- Every lint rule, with an allow comment.

Plus a probe on the desktop host: an app made with `audio` that plays nothing has the same wakeups over 60 s as one without the feature. zgx-audio's `device_probe idle` row says so for the engine (1 against 1).

## 8. Not in A1

Effects (EQ, delay, reverb, compressor, ducking) and `bus:duck` are A3; `ui.sound3d` and `ui.reverb_zone` are A2. A bus's pan, voice drives (an atomic per voice, like a bus's) and a `music:crossfade_to` helper come when a script needs them. `voice:fade` over two voices does a crossfade today.

## 9. As built (Nov 10 2026)

Where the build departs from sections 1 to 7, and why:

- **zgx-run's `audio` is off by default** for now (its `audio` feature turns on zgx-desktop's, zgx-script's and zgx-audio's `vorbis`). `zgx_run::play_through(host, cx)` wires a host to the app's engine.
- **The engine is made at a script's first call of `audio`** (`ScriptHost::set_audio` takes a function that makes it), not when the global is read: a script that names `audio` and calls nothing makes none. zgx-desktop keeps one per app (`WindowFactory::audio`), on the main thread.
- **Without the feature, `audio` is nil** and `sound =` an unknown property, with no message naming the feature: a message would be code in every build, and builds without the feature stay byte for byte as they were.
- **`sound =` takes `{ sound = click, gain, pitch }`** (the sound first works too), and a function choosing a sound as it's activated in place of a state (a state holds values, not handles). Its type is a union luau-lsp can tell apart only with `Sound`, `Voice` and `Bus` as extern types.
- **States a script reads are `ReadOnlyState`s** (`ready`, `error`, `playing`, `ui_sounds_enabled`), as the engine's other states are: `Readable` has no `get` a script can call.
- **The interface-sound setting is read as the engine is made and each time the app comes forward** (macOS's global `com.apple.sound.uiaudio.enabled` through CFPreferences, absent meaning on). No notification is known to be posted when System Settings changes it, and the key's dots make it a key path to `NSUserDefaults`' KVO; the setting can only change while another app is in front. Linux and Windows don't read it yet (on).
- **Suspend**: winit never suspends a desktop app, and zgx-audio has no pause; nothing is done (as section 2 says).
- **`audio.add_bus` without a parent is under `master`**; made again with a name that exists (a reload), it returns that bus.
- **Plugins**: their own bus `plugin:<name>` under `ui`, 8 voices with their streams among them (a ninth's voice never plays), no `bus`, `add_bus`, `prewarm` or `ui_sounds_enabled`, and no `bus` option. Their paths are confined to their directory with `within` (names and links), as images and modules are. A sound's decoded samples are held through the plugin's meter (`Meter::hold_native`) for as long as the plugin holds the sound; a plugin's sounds aren't in the host's cache.
- **Loading**: sounds decode on a `BackgroundPool` of two threads (parked while idle, 256 loads waiting at most). The app's cache lets go of sounds no script holds after a reload and after the host collects garbage, keeping the most recent 64 MB of them.
- **Values that aren't finite** (`0 / 0`) are passed over by every follower and binding, which keep the last finite value: a NaN goal is never reached, and a binding would step every frame for good.
- **`sound3d-no-max`** waits for `ui.sound3d` (A2).
- **Bindings step in zgx-script's audio module**, with the tween and spring of `anim::drive` written again there: a voice's or bus's binding outlives any component, which `anim::drive` needs, and changing `anim::drive` would change builds without the feature.
- zgx-audio's `Stats` gained `commands` (commands sent), which shows a bus's drive sends none.

## 10. Effects in Luau (Nov 10 2026)

The mixer's stock effects (zgx-audio's `effects`) on buses, built in `crates/zgx-script/src/audio/effects.rs`:

- `audio.low_pass`, `high_pass`, `peaking`, `delay`, `reverb` and `compressor` each take a table of parameters by name and answer an `EffectSpec`; `bus:add_effect(spec)` makes the engine's effect (`Bus::add_effect`) with the values each parameter has now, through the type's builders (no command per parameter), and answers an `AudioEffect`. `bus:duck(side, opts)` is `Bus::duck(&side, Duck)`.
- A parameter given a state or function is followed as a voice's gain is (one `EffectHandle::set` a change, a 10 ms glide); a binding steps on the display's frames (one `set` a frame while it moves, over the frame's interval). Literals ride in the constructor. `effect:set(name, value)` replaces what followed; `effect:fade` takes over from it; `effect:remove` takes it off.
- Effects are kept per load as voices are: a reload removes those the replaced script added (`EffectHandle::remove`), and the host's going removes all.
- Plugins reach their own bus as `audio.bus()` (nil for any name), with `PLUGIN_EFFECTS` (4) effects at most and no `duck`. What an effect holds is charged to the plugin's meter (`hold_native`) while it's on: a delay its line (`longest` seconds of stereo), a reverb its eight lines at their largest and 250 ms of pre-delay.
- `voice:crossfade(to, seconds, gain?)`: this voice's `stop_after` and `to`'s `set_gain` along `Curve::EqualPower`, both over `seconds`, taking the gains over from what followed them.
- Lint: `audio-effect-in-builder`.
