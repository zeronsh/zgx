# Images and canvas: image, svg, canvas

Pixels from files, vector icons, and drawing in script. Paths in `src` are relative to the script's directory. The same section covers what makes paint for these and other nodes: `ui.path` and `ui.linear` for canvases, `ui.bitmap` for masks, and `ui.shader` for shader fills.

```luau
ui.image(props: ImageProps) -> Node
ui.svg(props: SvgProps) -> Node
ui.canvas(props: CanvasProps) -> Node
ui.path() -> Path
ui.linear(x1: number, y1: number, x2: number, y2: number, colors: { read [number]: Color }) -> Brush
ui.bitmap(w: number, h: number, alpha: (x: number, y: number) -> number) -> Bitmap
ui.shader(source: string, opts: { fps: number? }?) -> Shader
```

## image

A PNG, JPEG or GIF (with zgx-gpu's `gif` feature, on by default), decoded once per `src` and kept while a node shows it. A GIF plays, as Rust's `animated_image` does: paused while it's off screen, clipped away, faded out or the window isn't shown (no timer, nothing drawn), its last frame kept once a finite one ends, and still on its first frame while the system asks for reduced motion (`ui.prefers.reduced_motion`). Another `src` changes what plays, and a still stops it. A GIF whose frames fit 4 MiB decoded keeps them (a spinner costs nothing to play); a larger one keeps its file and decodes each frame as it comes, so however long it is it holds a few frames (a 400 by 300 GIF of 300 frames, 137 MiB decoded whole: 0.5 MiB held, under 3 MiB at its peak, about 1% of a core while it plays at 50 frames a second). Each frame repaints the picture's box alone. Of those no node shows, the most recently shown are kept within the host's budget (`ScriptHost::image_cache`, 48 MiB by default), so a reload or a page shown again decodes nothing.

| Option | Type | Default | |
| --- | --- | --- | --- |
| `src` | `Reactive<string>`, or a `Bitmap` | required | a path (from the script's directory) or an absolute one; `"host:<name>"`, an image the app made (`ScriptHost::image`), following each one it replaces it with; a `Bitmap` shown as it is (`ui.bitmap`, a pasted image from `ui.clipboard.read_image()`); a state or function (of a path or a `Bitmap`) swaps the image |
| `fallback` | `Node` | none | shown in the image's place while a remote one (`https://…`) loads, and if it can't be shown (a remote one failed, a file gone or not a picture: never a script error): an avatar's initials |
| `on_load` | `function(size: { w, h })` | none | its picture's size in pixels, once known: a file's from its header, before it draws (nothing decoded); a remote one's once it has arrived. Once per `src` |
| `on_error` | `function(why: string)` | none | why its picture can't be shown: a file gone (a sent picture deleted, a kept copy pruned) or not a picture, a remote one that failed (`"… didn't load: HTTP 503"`). Without it, a warning (`ScriptHost::warnings`), with the script line naming the file |

A remote picture that fails (a dropped connection, a 5xx) is asked for again by itself while a node shows it: 5 seconds after, then doubling each time it fails in a row, up to 5 minutes. Its `fallback` shows meanwhile, and the picture replaces it once it comes. A plugin's request is its own, to the app's `fetch_policy` too: its failures and backoff hold up no one else showing the URL, and what comes is shared.
| `fit` | `"fill" \| "contain" \| "cover" \| "none" \| "scale_down"` | `"fill"` | how the pixels fill the box (CSS `object-fit`) |
| `image_rendering` | `"auto" \| "pixelated"` | `"auto"` | how its pixels are sampled drawn larger or smaller (CSS's `image-rendering`): `"pixelated"`, each point the nearest pixel, its edges hard: pixel art, a magnified screenshot, a QR code. Inherited, as CSS's is: said on a box, the pictures in it take it |
| `slice` | `{ top, right, bottom, left }` | none | the image drawn in nine pieces (a nine-patch, CSS's `border-image` with `fill`): insets into its pixels cut corners, edges and a middle, drawn as many points in from the box's edges; corners keep their size, edges stretch along their side, the middle both ways: a button's or a bubble's frame at any size. Edges wider than the box shrink together |
| `object_position` | `{ x, y }` | `{ 0.5, 0.5 }` | where the pixels lie in the box where they don't fill it (CSS `object-position`): fractions of the room to spare, so `{ 0.5, 0 }` keeps a portrait's top under `fit = "cover"` |
| `decode` | `"auto" \| "sync" \| "async"` | `"auto"` | when a PNG or JPEG file decodes, as HTML's `decoding`: with `"auto"` a picture up to 256 by 256 pixels decodes as it is first drawn, so an icon is in its first frame, and a larger one off the main thread, showing a frame or so later; `"sync"` decodes it as drawn whatever its size (that frame waits); `"async"` always off the main thread. It is the file's: images showing one file share its decoded picture, so the last to say wins |
| `dither` | `number` | none | drawn dithered as Zeron dithers its wallpapers, in dots this many points square on the screen's grid: each dot takes the colour at its centre, lit to that colour's full value where its brightest channel passes a 4x4 Bayer threshold, else dimmed to 8%. A vivid two-level dot picture of the image's own hues, worked out on the GPU as the image draws (one sample and a compare a pixel), so it follows the image as it moves, scales or crossfades, with nothing precomputed |
| `characters` | `number` | none | drawn as characters, as an ASCII art generator draws a photo: the screen in cells of a 5 by 7 pixel font (each font pixel this many points, a cell 6 by 8 of them), each cell a glyph of ` .-+xXS80#@` picked by the brightness at its centre and lit in that colour at full value, over the image a little dimmed. On the GPU as it draws, with no texture of glyphs |
| `label` | `string` | `""` | its accessible name |
| `w`, `h` | `Length` | the image's size in pixels, as points | |

With `characters`, its `shader_params` reveal it: `{ { front, band, ragged }, { dx, dy } }`, distances from the window's middle (offset by `dx`, `dy`, fractions of the window) measured out to a rounded box at 1 (the window's half-extents). Behind the `front`, the image, fading in pixel by pixel over the `band`'s depth (no cells popping) under its glyphs; across the band ahead of it, characters alone, scattered sparse and faint at its outer edge and denser and brighter inward, scrambling through the font as the front moves; beyond, nothing, so the picture dissolves into what is behind the window through its own characters. `ragged` (0 to about 0.3) scatters each cell's place in the band. Animate `shaderParam0` from 0 to have the picture materialise out of characters; `band` 0 (the default) shows all of it. Measured in the window, every image revealed with the same values shares one front, as they move.

## svg

An SVG, from a file or from markup, drawn as vectors at every scale; `tint` recolors it. In a box of other proportions than its own it is drawn whole and centred, keeping them (SVG's default `preserveAspectRatio`), so a wide mark in a square box is not squeezed.

| Option | Type | Default | |
| --- | --- | --- | --- |
| `src` | `Reactive<string>` | `src` or `code` | a path; a state or function swaps it, and may return `"code:" .. markup` |
| `code` | `string` | | SVG markup |
| `tint` | `Reactive<Color>` | none | every painted pixel in this color, keeping its alpha (a state or function re-tints); `"currentColor"` takes the text color the node inherits, as CSS's does, so an icon follows its button's `color` and `hover` color |
| `label` | `string` | `""` | its accessible name |
| `w`, `h` | `Length` | the SVG's size | |

## canvas

`draw(c, w, h)` paints the canvas's box, `w` by `h` points. It runs when the canvas first lays out, and again when its size or a state it read changes; an equal drawing keeps its pixels.

| Option | Type | Default | |
| --- | --- | --- | --- |
| `draw` | `(c: Canvas, w: number, h: number) -> ()` | required unless `each` | |
| `each` | a list state | none | items drawn one by one by `draw_item`, after `draw`, in the list's order. Each item's drawing is kept by the item: one not written since (`set_key`, `patch`, `push` and `remove` leave the others as they were) is not drawn again, so dragging one shape of thousands draws that one |
| `draw_item` | `(c: Canvas, item, w: number, h: number) -> ()` | with `each` | one item's drawing, from the item alone. It reads no state: what it read would show until its item is next written (`zgx lint` reports it). A size change draws every item again |
| `label` | `string \| state \| function` | none | what it shows, read by assistive technology as an image's name: a chart's summary (`"Price, now 143.9, high 150.1, low 128.2"`), followed as state. Without one, the canvas is decoration, out of the accessibility tree |
| `view` | state or function of `{ x, y, zoom }` | none | its drawing's pan and zoom: drawn at `zoom` times its size, then moved by `x`, `y`. Followed without calling `draw`: a pan or zoom places what it drew (a board, a map), so `draw` draws in its own coordinates and reads no view |
| `bg_grid` | `{ step, min_step, dot, lines, color }` | none | a grid under the drawing that follows `view`: a mark every `step` points (16), the step doubled while marks would show closer than `min_step` (0); each a square `dot` that many points across (1) or `lines` that wide; `color` (`"$border"`). Drawn in Rust with no call per mark, and marks keep their size as the view zooms |

| Method of `c` | |
| --- | --- |
| `c:rect(x, y, w, h, paint, radius?)` | a filled rectangle, its corners rounded by `radius` (0) |
| `c:circle(cx, cy, r, paint)` | a filled circle |
| `c:line(x1, y1, x2, y2, paint, width?, opts?)` | a stroked line, `width` 1; `opts`: `cap` (`"butt"`, `"round"`, `"square"`), `join` (`"miter"`, `"round"`, `"bevel"`), `dash` (`{ on, off, … }`), `dash_offset` |
| `c:fill(path, paint, rule?)` | fills a path; `rule` `"nonzero"` (the default) or `"evenodd"` |
| `c:stroke(path, paint, width?, opts?)` | strokes a path; `opts` as `line`'s, and `draw = t` (or `{ start, end }`) strokes only that fraction of it, as anime.js draws lines |
| `c:clip(path)` | clips what follows to the path, until `c:restore()` |
| `c:restore()` | ends the last clip |
| `c:text(text, x, y, opts?)` | a line of text at (`x`, `y`). `opts`: `size` (12), `color` (`"$text.primary"`), `weight`, `italic`, `tabular`, `font`, `align` (`"start"`, `"center"`, `"end"`: where `x` falls across it), `baseline` (`"top"`, `"middle"`, `"bottom"`: where `y` falls down it) |
| `c:measure_text(text, opts?)` | its width and height, for placing what goes around it: `local w = c:measure_text(label, { size = 11 })` |

Text on a canvas is shaped and drawn as any text is: sharp at any zoom, in the theme's fonts with their fallbacks, and following `view`. It sits over the canvas's shapes whatever order they were drawn in, so a shape drawn after it doesn't cover it, and `c:clip` doesn't cut it. Assistive technology doesn't read it: give the canvas a `label` that says what it shows (`label = "Requests, now 5,400, up 1.4%"`), and offer a chart's numbers as text or a table where they matter.

`paint` is a [color](properties.md) or a brush from `ui.linear(x1, y1, x2, y2, { colors })`, its colors spread evenly from (`x1`, `y1`) to (`x2`, `y2`). A color may name a theme token (`"$accent"`, `"$surface.raised"`), resolved in the canvas's theme while `draw` runs: the drawing follows it as it follows state, so a theme change draws it again. `ui.linear` called inside `draw` takes tokens too.

A path is built by chaining: `ui.path():move(x, y):line(x, y):quad(cx, cy, x, y):cubic(c1x, c1y, c2x, c2y, x, y):arc(cx, cy, r, start, sweep):close()`. `arc` angles are degrees, clockwise from +x. Once drawn, a path is fixed: it can be drawn again (make it once, outside `draw`), not extended. `:rect(x, y, w, h, r?)` and `:ellipse(x, y, w, h)` add a rounded rectangle or an ellipse as a closed subpath.

A canvas whose drawing is only solid fills and strokes of rectangles (rounded or not), circles and lines along an axis (`c:rect`, `c:circle`, `c:line`, or a path that is just one `:rect` or `:ellipse`), and solid strokes of straight lines running one way across (`:move` then `:line`s whose x never turns back, as a sparkline's or a chart's, any joins and caps), is drawn by the GPU as those shapes: nothing is rasterized or uploaded, so redrawing it as a view pans or zooms, or as a sparkline's data ticks, costs its shapes alone. Anything else in it (curves, other ellipses, lines that turn back or crowd more than 32 points into a few widths, gradients, patterns, dashes, clips), or the canvas faded as a whole (`opacity` below 1, an edge fade), rasterizes it as before.

A board that pans and zooms draws its content once, in its own coordinates, and gives the view: moving it draws nothing again. What keeps its size on screen as the view zooms (selection outlines, handles) goes in a canvas of its own over it, drawn in screen coordinates:

```lua
local view = ui.state("view", { x = 0, y = 0, zoom = 1 })
local shapes = ui.state("shapes", { { x = 10, y = 10, w = 40, h = 30 } })
return ui.overlay {
	w = 400,
	h = 300,
	bg = "$surface.sunken",
	on_wheel = function(e: WheelEvent)
		local v = view:peek()
		view:set { x = v.x - e.dx, y = v.y - e.dy, zoom = v.zoom }
		return true
	end,
	ui.canvas {
		w_full = true,
		h_full = true,
		view = view,
		bg_grid = { step = 16, min_step = 12, dot = 1 },
		-- Each shape drawn on its own and kept: moving one draws that one.
		each = shapes,
		draw_item = function(c, s)
			c:rect(s.x, s.y, s.w, s.h, "$accent", 4)
		end,
	},
}
```

## Masks and shaders


Any node can show its content through a bitmap's alpha, have its box filled by a WGSL function, or composite its content through one; see [properties](properties.md#paint):

| | |
| --- | --- |
| `ui.bitmap(w, h, fn)` | `w` by `h` pixels (1 to 4096 a side), white, each with the alpha `fn(x, y)` returns (0 to 1). A `Bitmap` can also be a full-colour image (`ui.clipboard.read_image`'s); a mask reads only its alpha |
| `mask = bitmap` or `mask = { bitmap, w = …, h = …, x = …, y = …, sweep = … }` | the content shows through the bitmap's alpha, drawn `w` by `h` (the bitmap's size) from (`x`, `y`), unsmoothed; `maskX` and `maskY` animate its place, and `sweep = { x, y, duration, frame_rate }` moves it by (`x`, `y`) every `duration` ms, over and over, on the engine's own clock (a text shimmer: no script, style pass or layout per step, only the masked node composites again; 4 steps a second while it doesn't show) |
| `ui.shader(wgsl, { fps })` | a fill: the source defines `fn fill(input: FillInput) -> vec4<f32>` (straight alpha), reading `input.p` (points from the box's top left), `input.size`, `input.time` (seconds), `input.scale` (pixels per point) and `input.params` (four `vec4`s); `fps` caps how often time repaints it |
| `bg_shader = shader`, `shader_params = { … }` | fill the box with it, over `bg` and under the border and children; parameters are colors (0 to 1) or lists of four numbers; `shaderTime` and `shaderParam0` to `shaderParam15` animate |
| `ui.layer_shader(wgsl, { fps })` | a layer shader: the source defines `fn layer(input: LayerInput) -> vec4<f32>` (straight alpha), reading `input.color` (the content's pixel there, straight alpha), `input.p`, `input.size`, `input.time`, `input.scale` and `input.params` as a fill does, and `input.radii` (the box's corners: top-left, top-right, bottom-right, bottom-left); `zgx_sample(p)` reads the content at another point, and `zgx_backdrop(p)` what lies behind the node there (the window under it, or inside an isolated group that group) |
| `layer_shader = shader`, `layer_params = { … }` | the node and its children paint once into a cached layer, and each pixel of the box passes through the shader as the layer composites (a foil sheen, a recolour, a ripple); past the box the layer shows as painted. `layerTime` and `layerParam0` to `layerParam15` animate it without repainting the content |
| `layer_clock = true`, `shader_clock = true` | the engine steps `input.time` on the window's frame clock, with no script per step and at most at the shader's `fps`, while the node shows and its window is presented; off screen, clipped away, faded out or hidden it holds no timer. With reduced motion it stays at 0: read `ui.prefers.reduced_motion` for a pose of your own |
| `ui.layer_shader(wgsl, { passes, images, geometry })` | a shader with [inputs](#passes-state-and-what-a-shader-reads): passes drawn before each frame and read by name, pictures, and where tagged boxes are. Each call is one running effect with its own textures |
| `geometry = "tag"` | this box's rounded rectangle, where it is on screen each frame, is read by shaders asking for `"tag"` |
| `layer_on = state`, `layer_params = function() … end` | while `layer_on` is false there's no shader, no layer and no clock, as if it weren't set; a function for `layer_params` is followed as state it reads changes, compositing again without repainting |
| `layer_bleed = n`, `layer_reach = n` | the shader runs `n` points past the box and can paint there (cut by the clips around the node, not its own), and its `zgx_backdrop` reads `n` points past that, holding at the edge beyond. A shader calling `zgx_backdrop` repaints when what it can read changes, and each one that a frame damages copies that part of the window, once |

## Example

```lua
local level = ui.state("level", 0.6)
local wave = ui.path():move(0, 30):cubic(30, 0, 60, 60, 90, 30):cubic(120, 0, 150, 60, 180, 30)
local glow = ui.shader([[
fn fill(input: FillInput) -> vec4<f32> {
	let t = input.p.x / max(input.size.x, 1.0);
	return vec4<f32>(0.2 + 0.6 * t, 0.4, 1.0 - 0.5 * t, 1.0);
}]])
-- A dither over the whole box: every other pixel shows.
local dots = ui.bitmap(180, 24, function(x, y)
	return if (x + y) % 2 == 0 then 1 else 0
end)
return ui.column {
	p = 24,
	gap = 12,
	ui.row {
		gap = 12,
		items = "center",
		ui.image { src = "photo.png", w = 48, h = 48, fit = "cover", rounded = 8, label = "Avatar" },
		ui.svg { src = "icon.svg", w = 20, h = 20, tint = "#5ea5ff" },
		ui.svg {
			code = "<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 2 2'><rect width='2' height='2'/></svg>",
			w = 12,
			h = 12,
			tint = "#e5534b",
		},
	},
	ui.canvas {
		w = 180,
		h = 60,
		draw = function(c, w, h)
			c:rect(0, 0, w, h, "#161b22", 8)
			c:rect(0, h - 6, w * level:get(), 6, ui.linear(0, 0, w, 0, { "#5ea5ff", "#a99cff" }))
			c:stroke(wave, "#ffffffcc", 2, { cap = "round", draw = level:get() })
		end,
	},
	ui.div { w = 180, h = 24, rounded = 6, bg_shader = glow },
	ui.div { w = 180, h = 24, bg = "#5ea5ff", mask = dots },
	ui.slider { label = "Level", value = level, w = 180 },
}
```

## Gotchas

- `ui.image` needs `src`; `code` is the SVG's (an image with only `code` fails to load). A missing or undecodable file is the file's doing, not the script's: it draws nothing (or the image's `fallback`), is told to `on_error`, and is otherwise a warning at the script line naming it, never an error, which would stop the window's reloads. So for SVGs and `bg_image`. An empty `src` (`""`, a picture not chosen yet) shows nothing and is no error, as the web's is, for images, SVGs and `bg_image`.
- `ui.image_size(src)` gives a picture's size in pixels from its file's header, without decoding it, to hold its place at its final size before it draws: `local w, h = ui.image_size("shot.png")`. Nothing for a file gone or not a picture, or a remote one not yet arrived (asked for then: a function reading it again answers once it has).
- Images and SVGs are decoded once per `src` and shared by every node showing it. Ones no node shows stay only while they fit the image cache's budget, the most recently shown first. A host that watches its scripts (`ScriptHost::watch`) also watches these files: saving an edited image reloads the scripts, state kept, and they show the new one.
- An image is as large as its pixels in points unless sized; give it a `w` and `h` (or `aspect` and one of them).
- `draw` runs again only for its size and the state it read; reading state through `peek` (or outside `draw`) does not redraw it. Keep it cheap: it runs in the script for every change it follows.
- A drawn path cannot grow (`a drawn path cannot be extended`): build paths whole, then draw them.
- `ui.shader` checks the WGSL when it is made: a source that does not compile is a script error with its line. A shader is compiled once per source and `fps`. Its pipeline compiles from then on, off the frame, on a thread of its own: declared ahead of its first draw (at a module's top, say), it is ready by then. A frame that draws it before it is ready doesn't wait: the box shows its `bg` colour meanwhile and the window draws again once the pipeline is ready (a compile can take 70 to 360 ms on macOS).
- A mask's bitmap is sampled without smoothing, which suits dithers and sprites; scale it with `w` and `h`, not by making it larger.

## Making effects

These are what an effect is made from, so an effect you write costs what it uses and nothing else:

- **Shape:** `input.radii` and `input.size` give the box, for a signed distance to its rounded corners. Nothing past the box runs without `layer_bleed`.
- **Time and motion:** `layer_clock` steps time without script. A one-shot writes when it started to `layer_params`, or animates a parameter with `anime.animate(el, { layerParam0 = … })`, and turns itself off with `layer_on` when done. A pool of them is yours to keep.
- **Seed, scale and motion:** a seed is a parameter; `input.scale` is pixels per point; `ui.prefers.reduced_motion` follows the system.
- **First draw:** `ui.layer_shader` compiles off the frame from when it's called, so declare it ahead of when it must show.
- **Frost:** `backdrop_blur` on the same node runs first, and `zgx_backdrop` reads it blurred.

What's not there:

- **Pointer:** hit testing doesn't follow a warp. A warped element's clicks land where it was laid out.
- **Reads back:** a shader's output goes to the screen and nowhere else: no script can read what it read.

### Passes, state and what a shader reads

A shader can declare passes: WGSL functions that draw textures before each frame showing it, at a size you choose, in floats if you need them, keeping their last result if you ask. Later passes and the shader read them by name. That is enough for a fluid, particles that remember, or a picture worked out once.

```lua
-- Worked out once: a noise picture. Then a state that eases toward it each frame.
local GROUND = [[
fn run(input: PassInput) -> vec4<f32> {
	return vec4<f32>(fract(sin(dot(input.pixel, vec2<f32>(12.9898, 78.233))) * 43758.5453));
}]]
local FLUID = [[
fn run(input: PassInput) -> vec4<f32> {
	return mix(zgx_fluid(input.uv), zgx_ground(input.uv), min(input.dt * 2.0, 1.0));
}]]
local fire = ui.layer_shader([[
fn layer(input: LayerInput) -> vec4<f32> {
	let f = zgx_fluid(input.p / input.size);
	return vec4<f32>(f.rgb, f.a);
}]], {
	passes = {
		-- Worked out once, then only read.
		{ name = "ground", shader = GROUND, scale = 1, once = true },
		-- A grid of its own, keeping its state from frame to frame.
		{ name = "fluid", shader = FLUID, size = { 480, 160 }, format = "rgba16float", feedback = true },
	},
	geometry = "cards",
})
return ui.div { w = 240, h = 80, layer_shader = fire, layer_clock = true }
```

| In `passes` | Default | |
| --- | --- | --- |
| `name` | required | an identifier: `zgx_<name>(uv)` samples its texture (linearly, `uv` from 0 to 1), `zgx_<name>_texel(xy)` reads a texel, `zgx_<name>_size()` is its size |
| `shader` | required | WGSL defining `fn run(input: PassInput) -> vec4<f32>`: what the pixel holds |
| `size`, `scale` | `scale = 1` | `{ w, h }` pixels, or this fraction of the box's pixels (drawn afresh when they change) |
| `format` | `"rgba8"` | or `"rgba16float"`, past 1 and below 0 |
| `feedback` | `false` | its own last result stays readable under its name, at the cost of a second texture. Without it, a pass reading itself reads nothing |
| `steps` | `1` | runs this many times a frame (`input.step` counts them), as a solver's iterations |
| `once` | `false` | draws once (again if its size changes) |

`PassInput` is `uv`, `pixel`, `size` (its texture's), `time`, `dt` (the time since it last ran, at most a twentieth of a second), `step`, `frame`, `scale`, `area` (the box in window points: x, y, w, h) and `params` (the shader's `layer_params`, the same four vec4s). A pass reads a later pass's last frame; every texture starts zero.

`images = { name = bitmap }` samples a picture the same way. `geometry = "cards"` gives the shader and its passes the rounded boxes of the nodes with `geometry = "cards"`, up to 64, where they are this frame through springs and transforms, with no script: `zgx_rect_count()`, `zgx_rect(i)` (x, y, w, h), `zgx_rect_radii(i)` and `zgx_rect_distance(p, i)` (signed, negative inside, `p` in window points). `zgx_window(p)` turns a shader's `input.p` into window points, so a shader can cut a moving row at a fixed line. Light that falls on cards is a shader over them reading their boxes. zgx-fx's gallery (`crates/zgx-fx/examples/gallery/app.luau`) has two in full: a trail, one feedback pass stamping moving boxes and fading, and a bloom, a quarter-size pass blurred across and down.

What it costs: nothing for a shader without inputs. A shader with them holds its textures from its first draw until it's dropped, and runs its passes only for frames that show it, so one with no `layer_clock` runs them when something repaints it. Its passes compile off the frame, and until they have, its layer shows as painted. If the node stops being drawn, its time stops and so does `dt`. Boxes moving doesn't itself redraw a shader: give one that follows them `layer_clock`.

Each `ui.layer_shader` call with inputs is one effect with one set of textures, and the nodes showing it share them. To give each row its own, make the shader in the row's component.

Output that adds light without reading the backdrop: return the colour with alpha as its strongest channel. Over a dark scene that adds as light does, with no copy of what's behind.

The [lava example](https://github.com/zeronsh/zgx/tree/main/crates/zgx-script/examples/lava) composes all of these with a real list: a floor worked out once, a stable-fluids fire in passes, light from the cards' boxes, and rows cut at a line as they sink.

### In Rust: passes of your own

`ShaderInputs::views` names GPU views a layer shader samples: your own wgpu passes, with the renderer's device and an encoder, so anything the device has, compute included, feeds a shader in its place in the frame. On their own, in a box of their own, a GPU view is that alone:

```rust
use zgx::compose::{gpu_view, prelude::*};
use zgx_gpu::viewport::{self, Viewport, ViewportFrame};

#[derive(Default)]
struct Fluid {
    // Your textures, kept from frame to frame, and your pipelines.
}
impl Viewport for Fluid {
    fn render(&mut self, frame: &mut ViewportFrame<'_>) {
        // Record as many passes as it takes into `frame.encoder`, the last
        // into `frame.target`, premultiplied, `frame.size` device pixels.
    }
    fn device_changed(&mut self) {
        // Drop what was made with the old device.
    }
}

fn fluid() -> View {
    let view = viewport::view(Fluid::default());
    view.animate(true); // or `view.invalidate()` when it changes
    gpu_view("A fluid", &view).w(320.).h(240.)
}
```

**What it costs:** a texture of its box's size while it shows, and your passes for each frame it's invalidated or animating. While it's neither, nothing but compositing that texture when something over or under it changes. An animating one holds the frame clock while it shows, and only then.

**How it composites:** as an image, on every renderer that draws through wgpu (Metal, Direct3D 12, Vulkan, GL): clipped, rounded, faded and transformed as any image is, in paint order, on whichever plane its place in the window puts it around native views. Direct3D 11 (opt-in) may hand it another device, which is what `device_changed` is for. An opaque one drawn whole and flat with nothing painted over it may present as a plane of its own where the window's frames go through layers (macOS): that's only a cheaper way to show the same pixels.

**What it can't do:** it reads nothing of the window behind it. A layer shader over it reads it as it reads anything painted. `crates/zgx-3d` is one, end to end.

## Glass

`ui.glass { clear, tint, radius, ... }` is a box of glass holding what you put in it. On macOS 26 it's the system's own `NSGlassEffectView`, between the window's planes, drawn by the window server from what lies under it; a window with none pays nothing. Everywhere else (Windows, Linux, older macOS) it draws a material measured against the system's, following the theme's light or dark and the system's reduced transparency, with the same script. It takes no pointer events of its own, and assistive technology hears its node.

| Option | Type | Default | |
| --- | --- | --- | --- |
| `clear` | `Reactive<boolean>` | `false` | the more see-through of the system's glasses |
| `tint` | `Reactive<Color>` | none | a colour it takes on |
| `radius` | `Reactive<number>` | `24` | its corners' radius in points |

```luau
return ui.glass { w = 300, h = 200, radius = 24, ui.text { "Over the glass" } }
```

In Rust, `zgx::glass::glass(GlassStyle::default)`. For the window itself, `WindowOptions::glass` with `Glass::new(GlassMaterial::Liquid)` puts the system's glass behind the whole window on macOS 26, and on older macOS and elsewhere its heads-up glass.
