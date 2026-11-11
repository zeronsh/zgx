# Sound: audio, and `sound =` on controls

Sounds, music and the buses they play through, mixed by [zgx-audio](../../crates/zgx-audio): every start, stop and change lands on an exact frame, nothing clicks, and a limiter on master keeps the sum below full scale. The Rust side of each call is in [design-audio-luau.md](../design-audio-luau.md).

**Where it is.** zgx-script's `audio` feature, off by default: `zgx-run` has it as its own `audio` feature (`cargo run -p zgx-run --features audio -- app/`), which also turns on zgx-desktop's for the device and Ogg Vorbis. A Rust host turns on zgx-script's `audio` and gives the host an engine (`ScriptHost::set_audio`; a desktop app `zgx_run::play_through`). Without the feature, `audio` is nil and `sound =` an unknown property.

**What it costs.** Nothing until a script calls `audio`: the engine is made then. Loading a sound, setting a bus's gain or muting one opens no device and touches nothing of the system's audio. The first play starts the output; a second after the last sound (fades and tails included) it stops, and the app is as idle as one without sound.

**When script runs.** A numeric option (gain, pan, pitch) or a flag (mute, solo) can be a literal, a state, a function of state, or a binding (`ui.tween`, `ui.spring`). A literal is set once. A state or function is followed: one change is one message to the mixer, with no script per audio block. A binding eases in Rust, one message a frame while it moves and none at rest, with no script per frame. A bus's gain is read by the mixer each block, so even a binding stepping every frame sends no message.

```luau
audio.sound(src: string, opts: SoundOptions?) -> Sound
audio.music(src: string, opts: PlayOptions?) -> Voice
audio.play(sound: Sound, opts: PlayOptions?) -> Voice
audio.bus(name: string?) -> Bus?
audio.add_bus(name: string, opts: { parent: string? }?) -> Bus
audio.now() -> number
audio.prewarm() -> ()
audio.stats() -> AudioStats
audio.ui_sounds_enabled: ReadOnlyState<boolean>
```

```luau
local click = audio.sound("assets/click.wav")        -- once, at the top: decoded off the main thread
local volume = ui.state("music_volume", 0.8)

audio.bus("music"):gain(ui.tween(volume))             -- the slider eases it; no script per frame
local score = audio.music("assets/dawn.ogg", { loop = true, fade_in = 2 })

return ui.column {
	gap = 8,
	ui.slider { label = "Music", value = volume },
	ui.button { sound = click, "Save" },              -- plays on activation, on the ui bus
	ui.button {
		on_click = function()
			audio.play(click, { gain = 0.5, at = audio.now() + 0.1 }) -- 100 ms from now, to the frame
		end,
		"Later",
	},
	ui.button { on_click = function() score:stop(3) end, "Fade the music" },
}
```

[`examples/sounds`](../../crates/zgx-script/examples/sounds) is a whole app: clicks on controls, a metronome on the mixer's clock and a pad under a slider.

## Sounds

`audio.sound(src, opts)` loads a sound whole: WAV (PCM 8, 16, 24 or 32 bits, float), and Ogg Vorbis where the app was built with zgx-audio's `vorbis` (zgx-run's `audio` has it). It returns at once with a handle. The file is read and decoded on a thread of its own, and `sound.ready` (a read-only state) turns true when it's done. Paths are relative to the script, like images. A sound is kept by its path and file time: a reload loads it again only if the file changed. One no script holds any more is let go after a reload (and as the app goes idle), but for the most recent 64 MB of them. Sounds decode on two threads that sleep while nothing loads.

- A play asked for before the sound is ready starts when it is, if that's within 250 ms of the ask. Later than that it's dropped and counted in `audio.stats().late`, so a click never plays long after its button.
- `sound.error` (a read-only state) holds why a sound couldn't be loaded, or nil. A sound that failed plays nothing.
- `sound.duration` is its length in seconds, once ready (else 0).
- `opts.preload = false` waits for the first play to start loading.

Load sounds once, at the top of a script or a module. Loading in a component or a row loads again for every instance (`zgx lint`: `audio-load-in-builder`). A sound over about ten seconds is decoded whole into memory: use `audio.music` for it (`audio-sound-too-long`).

## Playing

`audio.play(sound, opts)` plays a sound and returns its voice. `opts`:

| | | default |
| --- | --- | --- |
| `bus` | the bus it plays into, by name or handle | `"world"` |
| `gain` | 1 is as recorded | 1 |
| `pan` | -1 left to 1 right, as a balance: the near side stays whole | 0 |
| `pitch` | speed and pitch together: 2 is an octave up | 1 |
| `at` | a time on `audio.now()`'s clock: it starts on that exact frame | as soon as it can |
| `loop` | plays round until stopped | false |
| `loop_start`, `loop_end` | seconds into the sound: an intro, then the loop between them | the whole sound |
| `priority` | kept over lower priorities when 64 voices are playing (-128 to 127) | 0 |
| `fade_in` | seconds, or `{ seconds, curve = "linear" \| "equal_power" \| "exponential" }` | none (1 ms if the sound starts away from zero) |

`gain`, `pan` and `pitch` may be states, functions or bindings: they follow for as long as the voice plays. A value that isn't a finite number (`0 / 0`) is passed over: the sound keeps the last one it had.

`audio.music(src, opts)` streams a file as it decodes (half a second ahead, on one thread for every stream): for music, ambience and anything long. Its options are `play`'s, on the `music` bus by default (`loop_start` and `loop_end` aside: a stream loops whole). A stream loops without a seam. If its decoding falls behind it fades out and back in over 5 ms rather than clicking (`audio.stats().dry`).

When 64 voices play, a new one takes the place of the quietest of the lowest priority (with a 5 ms fade), or doesn't start if it's quieter and no higher.

**A reload** stops what the replaced script started, voices and streams, each with its fade, so music never plays twice; what a failed reload started stops instead, and the script before it plays on.

## Voices

| | |
| --- | --- |
| `voice:stop(seconds?)` | fades out and stops (5 ms if not given; a fade, never a cut) |
| `voice:fade(gain, seconds, curve?)` | moves its gain along a curve; it takes over from a state or binding following the gain |
| `voice:gain(value)`, `voice:pan(value)`, `voice:pitch(value)` | sets, or binds to a state, function or binding (replacing what followed before) |
| `voice.playing` | a read-only state: true until it has ended or been stopped |
| `voice:at(time)` | the same voice, for the next call to land on that frame: `voice:at(audio.now() + 1):stop()` |

`voice.playing` turns false when the voice ends, with no polling: the engine wakes the app when voices end. Keep a voice in a local or a table, not in a state: states hold values, not handles.

## Buses

Every voice plays into a bus, and every bus into `master`. `ui`, `music` and `world` exist from the start; `audio.add_bus(name, { parent = "world" })` makes more (32 at most), under `master` unless `parent` says. Made again (a reload), the bus there is returned.

| | |
| --- | --- |
| `bus:gain(value)` | the bus's gain: a number, a state, a function or a binding. Each block the mixer glides to the latest value over 10 ms |
| `bus:fade(gain, seconds, curve?)` | a fade along `"linear"`, `"equal_power"` (crossfades) or `"exponential"` (even in decibels); it takes over from what followed the gain |
| `bus:mute(value)`, `bus:solo(value)` | booleans, states or functions, over 5 ms. A solo silences every bus not on the way to or from a soloed one |
| `bus:at(time)` | the same bus, for the next call to land on that frame |

**The `ui` bus follows the user's setting.** With the system's interface sounds off (macOS: *Play user interface sound effects*), the `ui` bus is silent. Every other bus plays on: a game's sound isn't an interface sound. zgx-desktop reads the setting as the engine is made and each time the app comes forward (it's changed in System Settings, another app). On Linux and Windows it isn't read yet: interface sounds count as on. `audio.ui_sounds_enabled` is that setting as a read-only state. It's separate from `ui:mute()`: neither undoes the other.

## Effects

A bus runs its effects in order on its sum, before its gain: 8 at most. `audio.<effect> { … }` describes one, and `bus:add_effect` puts it on:

```luau
local music = audio.bus("music")
local muffled = ui.state("muffled", false)
local cutoff = ui.computed(function() return if muffled:get() then 400 else 20000 end)
local filter = music:add_effect(audio.low_pass { cutoff = ui.tween(cutoff, { duration = 600 }), slope = 24 })
audio.bus("world"):add_effect(audio.reverb { size = 1.2, decay = 2.5, wet = 0.25 })
music:duck(audio.add_bus("dialogue"), { depth = -9 })   -- music under dialogue
```

| | parameters | |
| --- | --- | --- |
| `audio.low_pass`, `audio.high_pass` | `cutoff` (Hz, 10 to 24000), `resonance` (Q, 0.1 to 20; 0.707 flat) | `slope = 12` or `24` dB an octave, set once |
| `audio.peaking` | `frequency`, `q`, `gain` (dB, -36 to 36) | a band of EQ |
| `audio.delay` | `time` (seconds), `feedback` (0 to 0.98), `wet`, `dry` | `longest`: the longest `time` (1 s, or `time` if longer; 10 at most), what its line holds |
| `audio.reverb` | `size` (0.25 to 2), `decay` (seconds to fall 60 dB), `damping` (0 to 1), `pre_delay` (up to 0.25 s), `wet`, `dry` | |
| `audio.compressor` | `threshold` (dB), `ratio`, `attack`, `release` (seconds), `knee`, `makeup` (dB) | |
| `bus:duck(side, opts)` | `depth` (dB, -60 to 0), `threshold` (dB `side` must pass), `attack`, `release` | this bus goes down while `side` sounds |

Every parameter takes a number, a state, a function or a binding, followed as a voice's gain is: one message to the mixer a change, one a frame while a binding eases and none at rest, never script per frame. A value outside its range is kept to it.

`bus:add_effect` and `bus:duck` return the effect:

| | |
| --- | --- |
| `effect:set(name, value)` | a parameter, by name: a number, or a state, function or binding it follows from now on |
| `effect:fade(name, value, seconds, curve?)` | moves it along a curve, taking it over from what it followed (a decibel parameter in straight steps whatever the curve) |
| `effect:remove()` | takes it off its bus, its tail cut |
| `effect:at(time)` | the same effect, for the next call to land on that frame |

A reload takes off the effects the replaced script added. Add effects once, at the top of a script: one added in a component or row is one more for every instance (`audio-effect-in-builder`). Custom DSP is Rust's (`zgx_audio::Effect`): no script runs on the audio thread.

`voice:crossfade(to, seconds, gain?)` fades a voice out (and stops it) while `to`, started silent (`gain = 0`), comes up to `gain` (1), along equal-power curves:

```luau
local next_track = audio.music("assets/night.ogg", { loop = true, gain = 0 })
score:crossfade(next_track, 3)
```

## Clock

`audio.now()` is the mixer's clock in seconds: the frame it will mix next, over its rate. Times passed to `at` are on it. It stands still while no sound plays (the device is stopped). Schedule from it, not from `ui.every`: a play from a timer lands on the timer's tick, a play `at` a time lands on its frame. Plays from a timer faster than 50 ms are flagged (`audio-play-timer`).

`audio.prewarm()` starts the output now, for an app that knows a sound is coming and would rather not wait for the device to start (on macOS about 7 ms, about 100 ms the first time in a process).

## `sound =` on controls

`sound = click` on a button, a checkbox, a menu item or any element with `on_click` plays the sound when it's activated: a press, Enter or Space, or assistive technology's activation. It plays on the `ui` bus at once, before the handler runs. `sound = { sound = click, gain = 0.6, pitch = 1.1 }` sets its options; a function answering a sound (or `false`) chooses one as it's activated; `sound = false` (or nil) plays nothing. A checkbox's sound plays after it toggles, as its `on_click` runs.

## Stats

`audio.stats()` returns `{ voices, stolen, dropped, late, dry, streams, underruns, running, starts, stops, peak }`: voices playing, voices stolen, plays that never started, plays and commands applied late (zgx-script's dropped late plays among them), times a stream ran dry, streams open, underruns the system reported, whether the output runs, its starts and stops, and the loudest sample sent.

## Plugins

A sandboxed plugin's `audio` plays only into a bus of its own (`plugin:<name>`, under `ui`, so the user's setting silences it), `audio.bus()` with no name, with 8 voices at most, its streams (`audio.music`) among them: a ninth's voice never plays. It may put 4 effects on its bus (no `duck`), their buffers (a delay's line, a reverb's) counted against its memory. Its sounds and music come from its directory or below, as its images and modules do (a path out of it, by `..`, an absolute path or a link, is an error that says nothing of what's there), or a file its grants let it pick. A sound's decoded samples count against its memory (`Limits::plugin_memory`): one past it fails to load, with `sound.error` saying so, and its sounds go with it. `audio.bus(name)` is nil to it; it has no `add_bus`, `prewarm` or `ui_sounds_enabled`, and naming a `bus` in its options is an error.

## Lint

| rule | flags |
| --- | --- |
| `audio-load-in-builder` | `audio.sound` or `audio.music` in a component or row body: it loads on every build |
| `audio-play-timer` | `audio.play` from `ui.every` under 50 ms, or from a per-frame callback: schedule with `at` |
| `audio-sound-too-long` | a literal path given to `audio.sound` whose file lasts over 10 s (its header read, nothing decoded): it's decoded whole, so stream it with `audio.music` |
| `audio-param-in-loop` | `:gain`, `:pan` or `:pitch` called with a number from `ui.every` or a per-frame callback: bind a state or a tween instead |
| `audio-effect-in-builder` | `bus:add_effect` or `bus:duck` in a component or row body: an effect for every instance |

`ui.sound3d`'s `sound3d-no-max` comes with 3D sound.
