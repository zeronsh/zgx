# Sound: audio, and `sound =` on controls

> **Spec, not yet built.** This page describes the Luau API that zgx-script's `audio` feature will provide. The engine under it, `zgx-audio`, is built and tested (#1942, #1949, #1956, #1962). The Rust side of every call is in [design-audio-luau.md](../design-audio-luau.md). Until the feature lands, `audio` is not a global.

Sounds, music and the buses they play through, mixed by [zgx-audio](../../crates/zgx-audio): every start, stop and change lands on an exact frame, nothing clicks, and a limiter on master keeps the sum below full scale.

**Where it is.** `zgx-run` has it. A Rust host turns on zgx-script's `audio` feature (`zgx-script = { version = "0.1", features = ["audio"] }`), and zgx-desktop's for the device. It's off by default. Without it, a script reading `audio` stops with an error naming the feature.

**What it costs.** Nothing until the first sound plays. Loading a sound, setting a bus's gain or muting one opens no device and touches nothing of the system's audio. The first play starts the output; a second after the last sound (fades and tails included) it stops, and the app is as idle as one without sound.

**When script runs.** A numeric option (gain, pan, pitch) or a flag (mute, solo) can be a literal, a state, a function of state, or a binding (`ui.tween`, `ui.spring`). A literal is set once. A state or function is followed: one change is one message to the mixer, with no script per audio block. A binding eases in Rust, a frame a refresh, with no script per frame. A bus's gain is read by the mixer each block, so even a binding stepping every frame sends no message.

```luau
audio.sound(src: string, opts: SoundOptions?) -> Sound
audio.music(src: string, opts: PlayOptions?) -> Voice
audio.play(sound: Sound, opts: PlayOptions?) -> Voice
audio.bus(name: string) -> Bus
audio.add_bus(name: string, opts: { parent: string? }?) -> Bus
audio.now() -> number
audio.prewarm() -> ()
audio.stats() -> AudioStats
audio.ui_sounds_enabled: Readable<boolean>
```

```luau
local click = audio.sound("assets/click.wav")        -- once, at the top: decoded off the main thread
local volume = ui.state("music_volume", 0.8)

audio.bus("music"):gain(volume)                       -- the slider moves it; no script per block
local score = audio.music("assets/dawn.ogg", { loop = true, fade_in = 2 })

return ui.column {
	gap = 8,
	ui.slider { value = volume, min = 0, max = 1 },
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

## Sounds

`audio.sound(src, opts)` loads a sound whole: WAV (PCM 8, 16, 24 or 32 bits, float), and Ogg Vorbis where the app was built with `vorbis` (zgx-run has it). It returns at once with a handle. The file is read and decoded on a worker thread, and `sound.ready` (a readable state) turns true when it's done. Paths are relative to the script, like images.

- A play asked for before the sound is ready starts when it is, if that's within 250 ms of the ask. Later than that it's dropped and counted in `audio.stats().late`, so a click never plays long after its button.
- `sound.error` (a readable state) holds why a sound couldn't be loaded, or nil. A sound that failed plays nothing.
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
| `priority` | kept over lower priorities when 64 voices are playing | 0 |
| `fade_in` | seconds, or `{ seconds, curve = "linear" \| "equal_power" \| "exponential" }` | none (1 ms if the sound starts away from zero) |

`gain`, `pan` and `pitch` may be states, functions or bindings: they follow for as long as the voice plays.

`audio.music(src, opts)` streams a file as it decodes (half a second ahead, on one thread for every stream): for music, ambience and anything long. Its options are `play`'s, on the `music` bus by default. A stream loops without a seam. If its decoding falls behind it fades out and back in over 5 ms rather than clicking (`audio.stats().dry`).

When 64 voices play, a new one takes the place of the quietest of the lowest priority (with a 5 ms fade), or doesn't start if it's quieter and no higher.

## Voices

| | |
| --- | --- |
| `voice:stop(seconds?)` | fades out and stops (5 ms if not given; a fade, never a cut) |
| `voice:fade(gain, seconds, curve?)` | moves its gain along a curve |
| `voice:gain(value)`, `voice:pan(value)`, `voice:pitch(value)` | sets, or binds to a state, function or binding |
| `voice.playing` | a readable state: true until it has ended or been stopped |
| `voice:at(time)` | the same voice, for the next call to land on that frame: `voice:at(audio.now() + 1):stop()` |

`voice.playing` turns false when the voice ends, with no polling: the engine wakes the app when voices end.

## Buses

Every voice plays into a bus, and every bus into `master`. `ui`, `music` and `world` exist from the start; `audio.add_bus(name, { parent = "world" })` makes more (32 at most).

| | |
| --- | --- |
| `bus:gain(value)` | the bus's gain: a number, a state, a function or a binding. Each block the mixer glides to the latest value over 10 ms |
| `bus:fade(gain, seconds, curve?)` | a fade along `"linear"`, `"equal_power"` (crossfades) or `"exponential"` (even in decibels) |
| `bus:mute(value)`, `bus:solo(value)` | booleans, states or functions, over 5 ms. A solo silences every bus not on the way to or from a soloed one |
| `bus:at(time)` | the same bus, for the next call to land on that frame |

**The `ui` bus follows the user's setting.** With the system's interface sounds off (macOS: *Play user interface sound effects*; GNOME's `event-sounds`; Windows' *No Sounds* scheme), the `ui` bus is silent. Every other bus plays on: a game's sound isn't an interface sound. `audio.ui_sounds_enabled` is that setting as a readable state. It's separate from `ui:mute()`: neither undoes the other.

## Clock

`audio.now()` is the mixer's clock in seconds: the frame it will mix next, over its rate. Times passed to `at` are on it. It stands still while no sound plays (the device is stopped). Schedule from it, not from `ui.every`: a play from a timer lands on the timer's tick, a play `at` a time lands on its frame. Plays from a timer faster than 50 ms are flagged (`audio-play-timer`).

`audio.prewarm()` starts the output now, for an app that knows a sound is coming and would rather not wait for the device to start (on macOS about 7 ms, about 100 ms the first time in a process).

## `sound =` on controls

`sound = click` on a button, a toggle, a checkbox, a menu item, a segmented control's option or a list row with `on_click` plays the sound when the control is activated: a press, Enter or Space, or assistive technology's activation. It plays on the `ui` bus at once, before the handler runs. `sound = { click, gain = 0.6, pitch = 1.1 }` sets its options. `sound = false` (or nil) plays nothing; `sound` can be a state.

## Stats

`audio.stats()` returns `{ voices, stolen, dropped, late, dry, streams, underruns, running, starts, stops, peak }`: voices playing, voices stolen, plays that never started, plays and sounds applied late, times a stream ran dry, streams open, underruns the system reported, whether the output runs, its starts and stops, and the loudest sample sent.

## Plugins

A sandboxed plugin's `audio` plays only into a bus of its own (`plugin:<id>`, under `ui`), with 8 voices at most. Its sounds count against its memory, and it can't reach other buses, the clock's `prewarm`, or the system setting.

## Lint

| rule | flags |
| --- | --- |
| `audio-load-in-builder` | `audio.sound` or `audio.music` in a component or row body: it loads on every build |
| `audio-play-timer` | `audio.play` from `ui.every` under 50 ms, or from a per-frame callback: schedule with `at` |
| `audio-sound-too-long` | a literal path given to `audio.sound` whose file lasts over 10 s: it's decoded whole, so stream it with `audio.music` |
| `audio-param-in-loop` | `:gain`, `:pan` or `:pitch` called with a number from `ui.every` or a per-frame callback: bind a state or a tween instead |
| `sound3d-no-max` | `ui.sound3d` with `loop` and no `falloff.max`: it never goes virtual (3D, with A2) |
