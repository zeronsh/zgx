# Video

`zgx-video` turns a zgx interface into a video. A composition is an ordinary view: its layout, text, shaders, blur, glass, images, canvas, lists and every animation system work in it unchanged. `zgx video render` draws it offscreen one frame at a time, with no window and no browser, and writes it as H.264, HEVC, ProRes 4444 with alpha, a PNG for each frame, or a GIF.

It is a crate of its own, and an app that doesn't depend on it doesn't link it. The only thing it adds to the engine is `zgx::clock`, below.

```sh
cargo install --path crates/zgx-video          # the `zgx-video` binary; `zgx video` hands off to it
zgx video render crates/zgx-video/examples/hello hello.mp4
zgx video studio crates/zgx-video/examples/showcase
```

## A composition

A directory holds `video.luau` (or `app.luau`), which returns its compositions. `require("@video")` is the API, and `require("@fx")` is [zgx-fx](../crates/zgx-fx/README.md)'s effects.

```lua
local video = require("@video")

return video.compositions {
	video.composition {
		id = "Hello",
		w = 1280,
		h = 720,
		fps = 30,
		duration = 120, -- frames
		props = { name = "zgx" },
		render = function(props)
			local f = video.frame()
			return ui.column {
				w_full = true,
				h_full = true,
				items = "center",
				justify = "center",
				bg = "#0d0a1f",
				ui.text {
					`Hello, {props.name}`,
					text_size = 96,
					color = "#fff",
					opacity = f:map({ 0, 12 }, { 0, 1 }),
					y = f:spring { from = 60, to = 0, damping = 12 },
				},
			}
		end,
	},
}
```

`w`, `h`, `fps` and `duration` may be functions of the props, so a composition's length can follow its data. `seconds` can replace `duration`. `schema = { name = { min, max } }` bounds a numeric prop's slider in the studio.

## Time

`video.frame()` is the frame, counted from the start of the sequence it is asked in, as state:

| | |
| --- | --- |
| `f:get()` | the frame, followed where it's read (in a function given to a property) |
| `f:seconds()`, `f:progress()` | seconds since the start; 0 to 1 over the sequence |
| `f:map(input, output, opts)` | a property: `video.interpolate` of the frame |
| `f:spring(opts)` | a property: `video.spring` at the frame |

`video.interpolate(x, input, output, { ease, clamp, left, right })` maps `x` piece by piece from rising `input` points to `output` numbers or `"#rrggbb(aa)"` colours. It holds at the ends unless `clamp = false`. `ease` is a function or a name from `video.eases` (`linear`, and `in`, `out` or `inOut` with `Quad`, `Cubic`, `Quart`, `Quint`, `Sine`, `Expo`, `Circ`, `Back`, `Elastic`, `Bounce`); `video.bezier(x1, y1, x2, y2)` makes a CSS curve.

`video.spring { frame, fps, from, to, stiffness, damping, mass, velocity, delay, duration, overshoot }` is a damped spring in closed form. Any frame stands on its own, so seeking or rendering a range gives the frame that playing through gives. `duration` stretches the spring to settle in that many frames, and `overshoot = false` holds it at its end.

`video.random(seed)` is a number from 0 to 1 that the seed alone decides.

**Everything else keeps the video's time as well.** While a stage renders, its thread runs on a virtual clock (`zgx::clock::VirtualClock`) that moves one frame at a time. These all follow it: `anime.animate`, `ui.tween`, `ui.spring`, `ui.loop`, layout animations, `ui.after`, `ui.every`, `layer_clock` and `shader_clock` shaders, GIFs, zgx-fx's effects, `ui.now`, `os.clock`, `os.time` (from 1 January 2000) and `math.random` (seeded). An animation started as a sequence mounts starts at that sequence's first frame, and a picture loaded from a file is decoded before the first frame that shows it. The same input gives the same frames, which is tested. Something real time has to answer (a `ui.fetch`, a remote picture) holds the frame: `local done = video.hold("prices")` returns a function to call when it has answered, and the render waits for it (up to `--hold-timeout`, 60 s).

## Sequences, series and transitions

`video.sequence { from, duration, name, … }` shows its children only from frame `from` for `duration` frames, and their time starts at `from`. A function child is built as the sequence appears and is given that time. Sequences nest, and their offsets add up. The sequence fills its parent unless it has `layout = "none"`, and other properties go on its box. Named sequences show on the studio's timeline.

`video.series { video.scene { duration, name, … }, transition, video.scene { … } }` plays scenes one after another. A transition between two scenes overlaps them by its duration, and `video.series_length(list)` is the total length. Transitions: `video.fade`, `video.slide { from = "right" }`, `video.zoom`, `video.wipe { angle, soft }`, `video.shader_transition { wgsl, params }` (a [layer shader](reference/media.md#making-effects) over the arriving scene, `params(p)` giving its parameters as it goes from 0 to 1, and `zgx_backdrop` reading the scene leaving), or `video.transition { duration, ease, enter = function(p) return props end, exit = … }`.

## Video files

`video.clip { src, from, duration, trim, w, h, volume, loop, … }` shows a video file within its sequence, from frame `from` for `duration` frames, skipping `trim` frames of the file. Each frame of the video shows exactly the frame of the file its time gives. The file is decoded by ffmpeg at the composition's rate and scaled to `w` × `h` (in points, times the render's scale), and the frame is decoded before it is drawn. Played on, the decoder streams; asked for a frame behind it, as a render of a range is, it starts again from there. A clip's other properties are an image's (`fit` is `"cover"` unless given). Its sound goes into the mix unless `volume = 0`.

## Sound

`video.audio { src, from, duration, trim, volume, fade_in, fade_out, loop }` places a sound file within the sequence it appears in; it adds no node. Sounds are mixed sample by sample into one stereo track at 48 kHz and muxed into H.264, HEVC and ProRes files. WAV is read directly. Other formats are read through ffmpeg, or through afconvert on macOS.

## Rendering

```
zgx video render <dir> [composition] <out> [options]
zgx video still <dir> [composition] <frame> <out.png>
zgx video list <dir>
```

The extension of `<out>` picks the format: `.mp4` is H.264, `.mov` ProRes 4444, `.gif` a GIF, and a directory or a path holding `%04d` gives a PNG for each frame. `--codec h264|hevc|prores|png|gif` overrides the extension. H.264, HEVC and ProRes go through ffmpeg (found on the PATH, or where Homebrew puts it), fed raw frames through a pipe and tagged Rec. 709. PNGs and GIFs need nothing.

| Option | |
| --- | --- |
| `--fps N` | frames per second of the file; the composition plays at its own speed with more or fewer frames |
| `--scale S` | pixels per point |
| `--frames A-B` | only these frames, which are the same as those frames of the whole |
| `--props JSON`, `--props @file.json` | props over the defaults |
| `--dataset rows.json` | one video for each row's props; `<out>` names them with `{index}` or a field, `{title}` |
| `--transparent` | a clear background (ProRes, PNG, GIF) |
| `--quality N` | constant quality, lower is better (16) |
| `--motion-blur N`, `--shutter F` | N samples blended per frame, spanning F of its time (0.5) |
| `--mute` | leave the sound out |
| `--concurrency N`, `-j N` | render N chunks of frames at once, each on a thread of its own with its own virtual clock, stage and encoder (the CPUs' count, at most 4, by default). A chunk moves to its first frame without drawing the ones before it, so it draws what a render of the whole draws there. PNGs number as one sequence, and an encoded video's chunks are joined without encoding them again, with the sound added once. A GIF renders in one chunk |
| `--every-nth-frame N` | keep one frame in N, the file's rate divided by N (a light GIF) |

On an Apple M-series Mac shared with other work, the showcase with 4-sample motion blur renders in 77–80 s in one chunk and 30–31 s in four, about 2.6× faster; without blur, encoding bounds it at about 1.3–1.6×. In an earlier, quieter run, the showcase (1920×1080, 60 fps, 1,980 frames, shaders, blur and glass in most of them) renders in 43 s with 4-sample motion blur, about 46 frames a second. At half scale without blur it renders over 500 frames a second, encoding included. Those are single runs.

`zgx video still <dir> [composition] <frame> out.png [--thumbnail WIDTH]` writes one frame, as a render of the whole would draw it, with nothing encoded: a poster, or with `--thumbnail` one no wider than `WIDTH` pixels. In Rust, `zgx_video::render_still(&options, frame, max_width)` returns it as a `Still` (`width`, `height` and straight RGBA `pixels`), and `Still::write_png` saves it.

## Studio

`zgx video studio <dir>` opens the compositions in a window. The window itself is a zgx app (`crates/zgx-video/studio/app.luau`) with a composition list, the preview, a timeline of the sequences shown so far with a playhead to drag, playback at real speed (Space; ← and → step a frame), a props panel that redraws as props change, a render button with progress, and reloading as the script is saved. Its frames come from a thread with a virtual clock of its own, through the same code a render uses, so the preview shows what a render writes. While it isn't playing, that thread waits for a message and does no work. `--screenshot out.png --frames N` draws the studio headless.

## In Rust

`zgx_video::compose` has the same pieces for views: `Clock` (`get`, `map`, `spring`, `progress`), `sequence(from, duration, |clock| view)`, `interpolate`, `spring`, and `render_view(&RustComposition { … }, &RenderOptions { … }, |clock| view, progress)`. `zgx_video::Stage` is the base: any view, mounted on a virtual clock with an offscreen renderer, stepped and drawn a frame at a time. `zgx_video::render` renders a script, and `ScriptStage` plays one frame by frame.
