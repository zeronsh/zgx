# zgx-3d: rendering primitives for games (design, Nov 10 2026)

Read on origin/main `e89ada127`. Nothing here is built yet. The design questions it raised are settled in [section 14](#14-decisions); the bar for merging anything is [section 2a](#2a-the-bar-among-the-fastest-renderers).

The goal is general primitives a AAA-quality scene can be built from, proven by a reference demo (a harbor at dawn, in rain) written only against the public API. Water, weather, terrain, foliage and the like stay in the demo. Audio is in [design-audio.md](design-audio.md).

## 1. Where zgx-3d is today

Facts the design starts from (`crates/zgx-3d/src/render.rs` unless said):

- **One forward pass into the view's texture.** Up to three passes a frame: the sun's shadow map (only when a caster or the sun moved), a "behind" copy with a mip chain (only when a mesh transmits), and the main pass. Colour is 4× MSAA resolving into the view's `Bgra8Unorm` target; depth is `Depth32Float`, 0 near, 1 far.
- **No HDR target.** Exposure, tone mapping and the sRGB encode run in the mesh's fragment shader. The "behind" copy glass reads is already tone mapped.
- **One WGSL string with tagged lines.** `wgsl(variant, samples)` assembles a shader from chunks; `Variant(u16)` keys the pipelines. Group 0 is the frame, 1 the material's maps, 2 the skin and morph palette, 3 the morph deltas.
- **Lights are a uniform array:** one sun, eight point or spot lights (the nearest to the camera's target), hemisphere or environment ambient (a CPU-prefiltered 128² cube and SH9). Only the sun casts shadows: one 2048² map, four taps.
- **Everything is redone each drawn frame:** the mesh list, the sort, the uniforms, the whole instance buffer. There's no frustum culling. Only the shadow map and resource caches persist.
- **A world asks for a frame only when a setter changes it.** It never animates by itself and ignores the frame's time.
- **The view seam** (`crates/zgx-gpu/src/viewport.rs`) hands a painter the device, queue, an encoder and the target. It says nothing about the device's features or limits.
- **Direct3D 11 draws no GPU views.** On that path wgpu runs on its noop backend and a view's box is left empty. The shared-texture view host that design-3d.md section 6 agreed was never built.
- **GL** runs with downlevel limits. Without storage buffers, skinned and morphing meshes draw in their bind pose.
- **Tests** read single pixels back and compare them with tolerances. There are no reference images anywhere in the repository.
- **Gate:** `3d-textured` and `3d-mixed` count `world_bench` (2,000 meshes, 30 frames) on wgpu's noop backend, so they see CPU cost only. Their known no-change spread is 0.06% and 0.08%. `inline_scopes.py` doesn't read zgx-3d.
- **CI** runs zgx-3d's pixel tests on Linux only (llvmpipe, `ZGX_REQUIRE_GPU=1`). The macOS and Windows jobs run none of them.
- **There's no size check** in the repository: no script compares binary sizes or symbols between two commits.
- **There's no iOS host on main.** The only iOS code is a stack-segment `cfg` in the core; the iOS work is a spike on a lane's branch, and nothing in CI builds or tests iOS.

Already done, so not listed again below: clearcoat, transmission, volume, IOR and specular (glTF's extensions), skins, morph targets, clip playback, picking, orbit controls.

## 2. Principles

1. **A world that turns nothing on draws exactly as today.** The new renderer is a list of passes; today's three passes are that list with nothing added. `3d-textured` and `3d-mixed` must not rise, and today's pixel tests must pass unchanged.
2. **Every feature is off until asked for,** and costs nothing while off: no texture, no pipeline, no branch in the per-mesh loop. Per-mesh state stays one lookup (`Variant`'s lesson: a field that hashes per run costs every scene).
3. **A world still draws only on change.** Each feature that moves by itself (particles, temporal accumulation, exposure adapting, clips, compute at a rate) says when it's settled; a world animates only while one isn't. See [section 4.4](#44-when-a-frame-is-drawn).
4. **Forward, not deferred.** Clustered forward keeps MSAA as an option, keeps transparency and custom materials in one shading path, and keeps the memory of a G-buffer out. A depth, normal and velocity prepass feeds the screen-space passes.
5. **One capability table, three tiers** ([section 3](#3-backends-and-tiers)). A feature states what it does on each tier. Nothing fails silently: `World::caps()` says what the device can do, and a feature asked for where it can't run is reported once.
6. **Primitives only.** The test for each addition: could two unrelated games use it unchanged? Water is not a primitive; "a material that can read the scene behind it and displace its vertices" is.
7. **All of it lives in zgx-3d.** zgx, zgx-gpu and zgx-desktop change only where the view seam must (capabilities, the D3D11 view host, an HDR-capable target if approved), each as a new cold function with the UI gate read cell by cell.

## 2a. The bar: among the fastest renderers

zgx-3d is built to match or beat the most performant renderers, and a primitive merges only when it's shown to be among the best. Concretely:

- **Same scene, same machine, same output.** Each primitive is measured against established renderers drawing the same scene at the same resolution and comparable image quality: Bevy (it's on wgpu too, so the comparison isolates the renderer from the backend), Filament and Godot's Forward+ renderer, and a published engine figure where one exists for the hardware. The scenes are public benchmark scenes (fetched by the harness, never committed) plus the harbor.
- **What's compared:** GPU time per frame by pass, CPU time and instructions per frame, memory, and frames drawn when nothing changes (where zgx should win outright: the others draw every frame).
- **The merge bar:** at equal image quality, the primitive's GPU and CPU cost ties or beats the best of the others on the M5 Max and the 4090, or its PR says by how much it loses, why, and what closes the gap. A primitive that loses without a reason stays on its branch.
- **Each arm proves it drew:** frames counted, pixels checked, as for every probe.

What that bar changes in this design, against a simpler plan:

1. **GPU-driven from the start.** Kept instance data with dirty-range uploads, GPU culling and indirect draws move from the last milestone into R0 and R3: rebuilding every instance on the CPU each frame is today's largest gap to the fast renderers.
2. **A depth prepass with reversed Z and equal-depth shading,** so the forward pass shades each pixel once.
3. **Fewer, fatter passes.** The whole display chain is one fragment pass; screen-space effects run at half resolution with one shared depth pyramid; passes that can share a render pass do (it also keeps tile memory on Apple GPUs).
4. **Pipelines compiled ahead:** wgpu's pipeline cache is turned on and persisted, and variants a scene will need are compiled off the frame, so the first frame with a new material doesn't hitch.
5. **Bindless textures where the device has them** (texture binding arrays), so draws batch across materials; the per-material bind groups stay as the fallback.
6. **Meshlets and a visibility buffer are measured, not assumed:** they're built if the comparison scenes show clustered forward losing to them at high triangle counts.
7. **Hardware ray tracing is a quality tier, not an afterthought** (see 2b): where the device has it, probe updates, reflections and contact shadows trace rays; the rasterised paths are the fallback everywhere else.

## 2b. The look: what an Unreal-class frame needs

Speed is half the bar. The other half is the picture: the demo should look the way a current AAA frame does, and Unreal Engine 5 is the reference for that look (as a reference for our own eyes, not for published numbers). Path-traced Cycles renders stay the ground truth for whether light is *right*; Unreal frames of the same scene say whether it reads as a modern game.

What makes that look, and what it asks of this plan beyond the first pass:

| What reads as "Unreal quality" | First pass (sections 6.x) | What closes the gap |
| --- | --- | --- |
| Bounce light and reflections that follow moving lights and objects | Rasterised probes, screen-space reflections, reflection probes | Ray-traced probe updates and reflections on devices with ray queries; a surface cache or screen-space GI where they're missing |
| Shadow detail at every distance, soft where it should be | Four cascades, local atlas, PCSS, contact shadows | Higher-resolution virtual or per-object shadow pages where the demo shows cascades running out |
| Geometric density with no popping | Culling, LODs with crossfade | Meshlets with a visibility buffer, built if the demo's close-ups show the limit |
| A stable, sharp image | TAA and our upscaler | The same, tuned against a supersampled reference until thin geometry, foliage-like detail and specular don't shimmer |
| Atmosphere | Physical sky, froxel fog, light shafts | Volumetric shadows from local lights, fog lit by GI |
| Filmic colour | Exposure, tone map, grading, bloom, depth of field, motion blur | Lens effects tuned by eye against references |
| Materials that hold up close | The glTF extension set, subsurface, decals | Detail normals and parallax through the custom material interface |

The order of work doesn't change: the first pass builds every row's left column. What changes is that GI and reflections start with a study of what wgpu exposes for ray queries on Metal and Direct3D 12, so the probe and reflection designs take ray-traced updates as a first-class path from the start, and that each primitive is judged against an Unreal frame of the same setup as well as its performance target. Which right-column items get built is decided with frames side by side once the first pass exists.

## 3. Backends and tiers

| Tier | Backends | What it has |
| --- | --- | --- |
| A | Metal (macOS, iOS), Direct3D 12, Vulkan | Compute, storage buffers and textures in every stage, indirect draws, float targets, BC or ASTC textures. Everything in this document. |
| A, through a view host | Direct3D 11 | zgx's window stays on D3D11; views render on a D3D12 device into a shared texture (design-3d.md section 6). Features as tier A. **Has to be built first**: today a view draws nothing here. |
| B | GL (Linux without Vulkan) | No compute assumed, storage buffers not assumed, limited targets. |

On tier B, proposed:

| Feature | Tier B behaviour |
| --- | --- |
| HDR target, post chain, bloom, grading, vignette, grain, exposure | Works (fragment passes only). Automatic exposure reads a downsampled mip instead of a compute histogram. |
| TAA, upscaling | Works (fragment passes). |
| Clustered lights | Works: clusters are built on the CPU on every tier, and read from a data texture where storage buffers are missing. |
| Cascaded and local shadows, contact shadows, GTAO, SSR | Works. |
| Fog, sky | Height fog and sky work. Froxel volumetrics are off (they need 3D storage textures); the scene falls back to height fog. |
| Custom materials | Works. |
| Compute passes, GPU particles, GPU culling, meshlets | Off. Stock particle emitters fall back to a CPU update capped at 16,384 particles; custom update functions don't run. Culling is on the CPU. `caps()` says so. |
| Probes | Baked probes work. Dynamic updates are off. |

Parity is checked as it is today: tier A backends against each other on software adapters where CI has them (WARP for D3D12, lavapipe for Vulkan), Metal on the macOS job, hardware on WING-PC, the M5 and M1, and personal-metal. `d3d11_parity` gains view-host cases. No check is loosened without a minimal repro showing the driver differs.

**iOS** can't be verified until an iOS host is on main ([decision 14](#14-decisions)). Until then the parity lane keeps zgx-3d building for `aarch64-apple-ios` and designs for it; no primitive is called done on iOS. When it's there, iOS is tier A (Metal) with tighter memory budgets ([section 11](#11-budgets)) and tile-based GPU rules: no pass reads a target it also writes, and passes that can share a render pass do, so tile memory isn't stored and reloaded between them.

## 4. The frame

### 4.1 Passes and the graph

A world's frame is a **compiled list of passes**. Each pass declares the resources it reads and writes by name; the list is ordered and its temporary textures are assigned from a pool when the world's configuration or size changes, never per frame. Per frame the renderer walks the compiled list. With every feature off the list is today's (shadows, behind, main), drawing straight into the target.

With features on, the full list is:

```
 1  compute (app passes marked "before the frame")      app buffers and textures
 2  cull (GPU, tier A) or CPU cull                       visible instances, indirect arguments
 3  shadows: sun cascades, local lights                  shadow atlases (kept between frames)
 4  depth prepass: depth, normal, roughness, velocity    prepass targets
 5  hi-Z                                                 depth pyramid (occlusion next frame, SSR, contact shadows)
 6  screen-space: GTAO, contact shadows, SSR trace       half-resolution targets
 7  sky look-ups, froxel fog (inject, integrate)         sky LUTs, froxel volume
 8  clusters (CPU) → upload                              light and decal lists
 9  opaque forward                                       HDR colour (lit, fogged)
10  sky and background
11  copy "scene behind" + mips                           what transmission and custom materials read
12  transparent forward, particles (sorted or OIT)       HDR colour; reactive mask
13  temporal: TAA or upscale                             history (kept between frames), output-size colour
14  post chain (HDR): depth of field, motion blur, bloom, app passes
15  exposure → tone map → grade (LUT) → vignette, grain, chromatic aberration → encode
16  post chain (display): app passes
17  write the view's target (premultiplied)
```

15 is one fragment pass whose parts are compiled in or out. 17 is folded into the last pass that runs, so a world with only bloom and tone mapping adds three passes to today's, not seventeen.

### 4.2 Targets

- **Scene colour** is `Rgba16Float` when the world has a background or a sky, and when it's transparent over its box (alpha has to survive). `Rg11b10Ufloat` is used for half-resolution intermediates that carry no alpha.
- **Depth** moves to reversed Z (1 near, 0 far) `Depth32Float` when any feature needing the prepass is on: distant precision matters for fog, SSR and large scenes. A world with nothing on keeps today's depth, so its cost and pixels don't move.
- **MSAA and TAA** exclude each other per world: `Antialiasing::Msaa(4)` (today's default), `Taa`, or `None`. Under MSAA the screen-space passes read resolved depth.
- **Output** stays the view's `Bgra8Unorm` target, premultiplied, composited by zgx as an image. Display HDR (EDR on macOS, scRGB on Windows) needs a float view target in zgx-gpu and is left out of this plan ([decision 7](#14-decisions)).

### 4.3 Resource lifetimes

| Kind | Examples | Made | Dropped |
| --- | --- | --- | --- |
| Temporary | HDR colour, prepass targets, bloom chain | When the compiled list first needs them; shared between passes whose lifetimes don't overlap | When the host's idle trim runs (`Viewport::rest`: 1.5 s after the window's last present), on resize, on `device_changed`. A still world in a window that other UI keeps presenting isn't trimmed and keeps its temporaries: accepted until a measurement shows it matters. |
| History | TAA colour, previous depth, exposure, froxel history | First frame | With temporaries, and marked invalid on a camera cut (`World::cut()`), a resize, or a gap longer than 0.25 s of frame time |
| Kept | Shadow atlases, probe textures, sky LUTs, cluster buffers | When the feature turns on | When the feature turns off or the world drops; redrawn only when their inputs change |
| App's | Compute buffers and textures, particle state, material parameters | By the app's call | When the app drops its handle |
| Assets | Geometry, textures | On first draw | As today after 240 drawn frames unused, plus a byte budget ([section 9](#9-assets)) |

All temporary and history textures count against one per-world byte budget reported by `World::memory()`. After a world's box closes and its handles drop, the process's footprint returns to its level before the world was made: a test opens and closes the demo's scene ten times and reads `footprint` from outside, and an in-process counter of live GPU bytes must read zero.

Known trap, kept from the 3D lane's notes: on Metal, any blit into a texture opens driver blocks of 8 MB each. Pass targets are written by render passes only; uploads go through `zgx_gpu::viewport::texture_with_levels`.

### 4.4 When a frame is drawn

A world draws when a setter changes it, as today, and while any of these is unsettled:

| Feature | Unsettled while |
| --- | --- |
| A clip, an animator | It plays |
| Particles | Any emitter has live particles or is emitting |
| TAA | Fewer than 8 jittered frames have accumulated since the last change (then the image has converged and the world stops) |
| Automatic exposure | The adapted value is more than 1/64 stop from its target |
| Compute pass with a rate | It's enabled |
| Custom material reading `time` | It's on a visible mesh (`time` is opt-in per shader; a shader that never reads it never animates) |
| Shadow cascades, probes updating over several frames | Updates are queued |

It uses `GpuView::animate`, so frames stop while the box is offscreen, clipped away, transparent or its window hidden, as views already do. A still harbor with no rain and no moving water settles in 8 frames and then draws nothing. The idle test counts frames and process wakeups over 10 s.

**Time** comes from the frame (`ViewportFrame.time`), never the wall clock, so zgx-video's virtual clock renders a world deterministically: particles, noise and TAA jitter are functions of frame time and a frame index.

### 4.5 With the UI

Unchanged from design-3d.md: a view renders into its own texture, in an encoder submitted before the window's frame on the same queue; a new frame damages its box only; UI changes over a still world reuse its texture and run no pass.

- UI over a view is composited by zgx after the world's last pass, so UI never enters TAA history, motion blur or depth of field.
- Upscaling renders the world below the box's device size and writes the box's size; the UI never sees the internal size.
- An opaque world on its own layer (`set_opaque`, macOS) gets pass 17 written into the drawable.
- A world's box that resizes every frame (a split being dragged) would remake its targets every frame. Targets are sized in steps of 64 pixels and drawn with a viewport, so a drag reuses them.

## 5. Extension interfaces

Four public interfaces let apps add what the engine doesn't have. Each has a **stability promise**, stated per interface and repeated in its reference doc. In 0.x, "stable" means: not broken in a patch release, and broken in a minor release only with a changelog entry and a version check that names the change.

### 5.1 Custom materials

A custom material is WGSL functions plugged into the standard lit pipeline. Shadows, clusters, fog, probes, TAA velocity and tone mapping keep working around them.

```rust
let water = world.add_shader(MaterialShader::new(include_str!("water.wgsl"))
    .param("wind", Param::Vec2)            // app uniforms, by name
    .param("foam", Param::Texture)
    .param("waves", Param::Buffer)         // a compute pass's output
    .reads_scene()                         // may sample the scene behind it
    .animated());                          // reads time: keeps the world drawing while visible
world.set_shader(mesh, Some(water));
world.set_param(water, "wind", [0.3, 0.1]);
```

```wgsl
#zgx material 1
// Any of the three may be left out.
fn displace(v: VertexIn) -> Displaced {            // vertex stage; also runs in shadow and prepass
    var out: Displaced;
    out.position = v.position + vec3f(0., wave(v.world.xz, frame.time), 0.);
    out.normal = v.normal;
    return out;
}
fn surface(s: SurfaceIn) -> Surface {              // fragment stage
    var m = surface_default(s);                    // what the mesh's Material and maps give
    let behind = scene_color(s.screen_uv + refraction(s), 0.);
    m.base_color = mix(behind, m.base_color, 0.6);
    m.roughness = 0.05;
    return m;
}
fn opacity(s: SurfaceIn) -> f32 { return 1.; }     // for cut-outs and shadows of cut-outs
```

**Stable inputs** (`#zgx material 1`):

| Name | What |
| --- | --- |
| `frame.time`, `frame.delta`, `frame.index` | Frame time in seconds, the step, a counter |
| `frame.camera`, `frame.view`, `frame.projection`, `frame.size` | Camera position and matrices, target size |
| `VertexIn`: `position`, `normal`, `uv`, `uv1`, `color`, `world`, `instance` | Object-space vertex, its world position, the instance's four app floats |
| `SurfaceIn`: `world_position`, `normal`, `geometric_normal`, `uv`, `uv1`, `color`, `view`, `depth`, `screen_uv`, `front_facing`, `instance` | Fragment inputs; `depth` is linear view depth in metres |
| `Surface`: `base_color`, `metallic`, `roughness`, `normal`, `emissive`, `occlusion`, `opacity`, and the layers (`clearcoat`, `transmission`, `sheen`, …) | What the lit pipeline shades |
| `scene_color(uv, lod)`, `scene_depth(uv)` | The opaque scene behind (pass 11) and its linear depth |
| `params.<name>` | The app's uniforms, textures and buffers |
| `surface_default(s)`, `noise2/3`, `hash` | Helpers |

**Promise:** the names and meanings above. Fields are added, never removed or redefined, within version 1. Not promised: bind group numbers, the engine's own uniform layout, anything not in the table. A shader that reaches for them isn't supported.

**Costs kept off other meshes:** a shader's id joins the pipeline key beside `Variant`, compared only for meshes that have one. A mesh without a shader takes today's path.

**Errors** are reported with the app's line numbers (the prelude's lines are subtracted), at `add_shader`, off the frame. A shader that fails draws its mesh with the stock material in magenta in debug builds.

Water, wetness and puddles are demo materials on this interface. Wetness needs the surface *under* a puddle to change too, so a shader can also be set **world-wide as a layer** (`World::add_surface_layer(shader)`): its `surface` runs after every lit mesh's own, given that mesh's `Surface`. That's the general form of "rain makes everything wet"; it's off unless a layer is added, and it's a pipeline variant, not a per-mesh branch.

### 5.2 Post passes

```rust
let pass = world.add_post(PostPass::new(include_str!("heat.wgsl"))
    .stage(PostStage::Hdr)                 // or Display, after tone mapping
    .after(Stock::Bloom)                   // order among stock passes
    .param("strength", PostParam::Float));
```

```wgsl
#zgx post 1
fn post(p: PostIn) -> vec4f {              // p.uv, p.size, frame.*
    return color(p.uv + offset(p)) * params.strength;   // color(), depth(), velocity()
}
```

The stock passes (bloom, depth of field, motion blur, exposure, tone map, grade, vignette, grain, chromatic aberration) are ordinary entries in the same ordered chain with fixed default positions; apps turn them on, set them, and order their own passes around them.

**Promise:** `PostIn`, `color`, `depth`, `velocity`, the two stages, and that an HDR-stage pass receives scene-linear, pre-exposure colour. Not promised: the stock passes' internal targets.

A lower level exists for what a fragment function can't express: `RenderNode`, a Rust trait given the wgpu device, an encoder and named resources. **Promise: none beyond wgpu's major version**; it's marked unstable in its docs.

### 5.3 Compute and GPU data

```rust
let state  = world.add_buffer(GpuBuffer::storage::<Wave>(256 * 256));
let height = world.add_texture(GpuTexture::storage(256, 256, Format::Rgba16Float));
let fft = world.add_compute(Compute::new(include_str!("fft.wgsl"))
    .entry("step")
    .buffer("state", &state, Access::ReadWrite)
    .texture("height", &height, Access::Write)
    .dispatch([32, 32, 1])
    .every_frame());                       // or .on_demand(), .at_rate(30.0)
world.set_param(water, "waves", &height);  // feed a material
let read = state.read(0..64);              // a future; resolves 1–3 frames later
```

- **Feeds:** material and post parameters, particle emitters, geometry (a buffer as instance transforms, or vertices pulled from a storage buffer), and indirect draw arguments (`Draw::indirect(&args)`).
- **Order:** passes run in the order added, at one of three points (before the frame, before transparents, after the frame). Dependencies are by resource access; a cycle is an error at `add_compute`.
- **Readback** is asynchronous and budgeted: 64 KiB a frame by default (`World::set_readback_budget`), queued beyond that, never stalling the frame. Occlusion and physics queries in apps go through this.
- **Budgets:** app buffers and textures count in `World::memory()`, under an app-set cap (256 MiB by default); beyond it `add_buffer` returns an error.
- **Tier B:** `add_compute` returns `Unsupported`; `caps().compute` is false.

**Promise:** the builder API, binding by name, the three points in the frame, and readback's behaviour. WGSL here is plain WGSL: wgpu's and naga's limits apply and are the app's to follow.

### 5.4 Particles

An emitter is a compute update plus a draw, both built on 5.1 and 5.3, with stock modules for the common cases.

```rust
let rain = world.add_emitter(Emitter::new(200_000)
    .spawn(Spawn::box_([-40., 30., -40.], [40., 32., 40.]).rate(60_000.))
    .lifetime(0.6..0.9)
    .velocity([0., -22., 0.], 1.5)
    .force(Force::Gravity).force(Force::Wind(wind)).force(Force::Drag(0.1))
    .collide(Collide::Depth { bounce: 0.0, on_hit: Event::Spawn(&splashes, 3) })
    .draw(Shape::Streak { length: 0.04 }, Blend::Alpha)
    .soft(0.2).lit(true).receive_shadows(true));
```

- **Stock modules:** spawn shapes (point, box, sphere, cone, mesh surface, another emitter's events), rate and bursts, lifetime, velocity, colour and size over life, forces (gravity, drag, wind, curl noise, attractors, a vector-field texture), collision against the depth buffer (bounce, stick, die, emit an event), and sub-emitters on events.
- **Drawing:** billboards, velocity streaks, meshes, ribbons later. Soft against depth. Lit from the clusters per vertex (cheap) or per pixel, receiving cascade shadows and fog.
- **Transparency:** weighted blended order-independent transparency by default (no sort, one extra target), with a per-emitter GPU sort for the cases it handles badly (dense, opaque-looking smoke). [Decision 6](#14-decisions).
- **Temporal:** particles write the reactive mask, so TAA doesn't smear them.
- **Custom:** `.update(wgsl)` replaces or follows the stock update (`fn update(p: ptr<function, Particle>, e: EmitterIn)`), and `.material(shader)` draws through 5.1.
- **Settled** when no particle lives and nothing emits; a burst that ends lets the world go idle.

**Promise:** the `Particle` fields (`position`, `velocity`, `age`, `life`, `size`, `color`, `seed`, and four app floats), `EmitterIn`, and the modules' meanings.

## 6. The stock primitives

Each is off by default and set on the world (or on a light, a mesh, a volume). Techniques are the proposal; where an established alternative exists it's named so the choice can be argued.

### 6.1 Post chain

| Pass | Technique | Notes |
| --- | --- | --- |
| Bloom | 13-tap downsample, tent upsample chain, mixed back by a small weight (no threshold) | Energy conserving: the result is a lerp, not an add. Karis average on the first level against fireflies. |
| Exposure | Manual stops (today's), or automatic: a luminance histogram in compute (tier A) or a mip average (tier B), adapting with separate up and down rates | Metering mask and compensation settings. |
| Grading | White balance, lift/gamma/gain, saturation, then a 3D LUT (32³ or 64³ from a `.cube` file or strip image) | Applied after tone mapping, in display space, or before it in log space: per LUT. |
| Depth of field | Circle of confusion from a physical camera (aperture, focus distance, sensor), half-resolution gather with separate near and far layers | Bokeh shape from the aperture's blade count. |
| Motion blur | Velocity from the prepass (camera and object, skinned and displaced included), tile max, gather along it | Shutter angle. |
| Vignette, grain, chromatic aberration | In the final pass | Grain is a function of frame index, so video renders repeat. |

### 6.2 Anti-aliasing and upscaling

- **TAA:** Halton jitter, velocity reprojection with closest-depth dilation, history clamped in YCoCg by neighbourhood variance, bicubic history sampling, and a **reactive mask** that transparents, particles and custom materials write so their pixels lean on the current frame. Disocclusions restart their history.
- **Upscaling:** the same pass resolving to a larger output (render scale 0.5–1.0), with a sharpening step. It's zgx's own, in WGSL, so it runs on every tier. Vendor upscalers (MetalFX, FSR, DLSS) aren't planned: each is one platform's binary or licence.
- **MSAA** stays, and is the default for worlds that don't turn TAA on.
- **Exit test for "no ghosting":** a moving high-contrast edge, a particle burst and a transparent pane over a moving background, each compared against a 64-sample supersampled reference of the same frame.

### 6.3 Lighting and shadows

- **Clustered forward:** a 16×9×24 view-space grid; lights and decals are binned on the CPU each frame a light or the camera moves (hundreds of lights cost tens of microseconds; a compute build is measured against it before being built). Lists go up in one buffer. The 8-light uniform path stays for worlds with 8 or fewer lights and no clusters asked for.
- **Light types:** point and spot (today's, now with physical units: lumens, candela, and a radius for soft falloff), **rect and disc area lights** by linearly transformed cosines, with textures on rect lights later. **Emissive meshes** light the scene through probes and bloom, not as analytic lights; a mesh that must cast sharp light gets a rect light placed on it. That's a stated limit, not a hidden one.
- **Sun shadows:** up to four cascades in one atlas, fit to the view with texel snapping so they don't swim, far cascades redrawn at a lower rate, and each redrawn only when a caster in it or the sun moved (today's rule, per cascade).
- **Local shadows:** spot lights get a tile, point lights six, in a second atlas with a budget (tiles and bytes); lights are ranked by screen size, and those past the budget go unshadowed, reported in stats.
- **Softness:** PCF by default (today's four taps, or a wider rotated disc), PCSS per light for contact-hardening.
- **Contact shadows:** a short screen-space march along the light direction against the depth pyramid, for the sun and the most important local lights.

### 6.4 Global illumination and reflections

- **Irradiance volumes:** the app places `ProbeVolume`s (a box and a spacing). Each probe stores irradiance as octahedral maps plus depth moments for visibility, the layout DDGI uses. **Updates are rasterised:** zgx has no ray tracing on its backends, so a probe is refreshed by drawing six small faces, a budget of probes a frame, on demand (`volume.bake()`) or continuously. Baked results can be saved and loaded as an asset. This is a real difference from ray-traced DDGI: updates are slower to spread and cost draw calls. [Decision 5](#14-decisions).
- **Reflection probes:** placed boxes, captured and prefiltered on the GPU into a cube array, box-projected for parallax, blended by influence. The environment is the last fallback.
- **Screen-space reflections:** a hierarchical-depth trace at half resolution on the previous frame's colour, resolved with roughness, falling back per pixel to probes and then the environment where the ray leaves the screen.
- **Ambient occlusion:** GTAO at half resolution, depth-aware upsample, temporally filtered under TAA. It multiplies indirect light only, and bent normals are kept for specular occlusion.
- **The environment's prefilter moves to the GPU** (it's 11–16 ms on a CPU thread today), so a sky that changes can relight the scene in a few frames.

### 6.5 Participating media and sky

- **Height and distance fog:** analytic, in the forward shader, a few instructions; the fallback everywhere.
- **Volumetric fog:** a 160×90×64 froxel grid: densities injected from **density volumes the app places** (box, sphere, height slab; uniform, noise, or a custom density function through 5.1's parameter mechanism), lit from the clusters and cascades with a phase function, reprojected temporally, integrated front to back, and sampled by opaque and transparent shading alike. Light shafts fall out of the sun's shadowing.
- **Sky:** a physically based atmosphere from look-up tables (transmittance, multiple scattering, sky view, aerial perspective), after Hillaire 2020, with a sun disc. The sky can be the world's background and its environment light.
- Clouds, rain sheets and weather are not here.

### 6.6 Materials

- **glTF extensions still missing:** sheen, anisotropy, iridescence, emissive strength, and dispersion if cheap. Each joins `Variant` only if it stays one byte's lookup per run; otherwise `Variant` widens once, measured on both 3D cells first.
- **Subsurface scattering:** a profile per material (scatter colour and radius). Screen-space separable diffusion for skin-like surfaces, plus thickness-based back-lighting for thin ones (leaves, wax, cloth), which needs no extra pass.
- **Alpha to coverage** for cut-outs under MSAA; under TAA, cut-outs dither by a jittered threshold.
- **Decals:** boxes the app places, binned into the clusters with the lights and applied in the forward shader to base colour, normal and roughness. Works with MSAA and transparency, and needs no G-buffer.

### 6.7 Geometry at scale

Today the whole instance buffer is rebuilt every drawn frame and nothing is culled. In order:

1. **Kept instance data:** instances live in a GPU buffer updated by dirty ranges. A world of 100,000 still objects uploads nothing when the camera moves.
2. **CPU frustum culling** over a bounding-volume hierarchy, on every tier.
3. **GPU culling (tier A):** frustum and hi-Z occlusion in compute against last frame's depth pyramid, with a second pass for what was newly revealed, writing indirect arguments.
4. **Levels of detail:** `Geometry::with_lods`, chosen by projected size, changing by dither crossfade (clean under TAA).
5. **Meshlets (tier A):** clusters built when a geometry is made, culled per cluster in compute into a compacted index buffer. Built last, and only if the demo's cranes and containers show a need the first four don't meet.
6. **Heightfields and procedural meshes:** `Geometry::heightfield(samples, spacing)`, and `Geometry::dynamic` whose vertices and indices update in ranges or come from a compute buffer. Terrain and foliage placement are the app's.

### 6.8 Animation

On the CPU, as posing is today: an `Animator` holding **layers**, each with a weight, a joint mask and a blend mode (override or additive), clips crossfading within a layer, clip events, **root motion** taken from a named joint and returned to the app, and an **IK hook**: a function called with the pose after blending and before skinning, with two-bone and look-at solvers supplied. A world with no animator pays nothing.

## 7. Rust API

In the style of today's `World` setters: plain data in, a frame only on change.

```rust
world.set_antialiasing(Antialiasing::Taa);
world.set_render_scale(0.67);
world.set_post(Post { bloom: Some(Bloom { intensity: 0.04, ..Default::default() }),
                      exposure: Exposure::Auto { compensation: -0.3, ..Default::default() },
                      grade: Some(Grade::lut(lut)), ..Default::default() });
world.set_sky(Some(Sky::physical().sun(direction, 20.0)));
world.set_fog(Some(Fog::height(0.02, 12.0)));
let mist = world.add_volume(Volume::box_(min, max).density(0.4).noise(0.6, 3.0));
world.set_shadows(Shadows::cascades(4).softness(Softness::Pcss));
let lamp = world.add_light(Light::spot(position, direction).lumens(4000.).cone(30., 40.).shadows(true));
let probes = world.add_probes(ProbeVolume::new(min, max, [8, 4, 8]).dynamic(4));
world.add_reflection_probe(ReflectionProbe::box_(min, max));
world.set_ambient_occlusion(Some(Gtao::default()));
world.set_reflections(Some(Ssr::default()));
world.caps();      // Caps { tier, compute, storage_vertex, indirect, compressed: Bc | Astc | None, … }
world.stats();     // draws, culled, lights, shadow tiles, GPU ms by pass (where timestamps exist)
world.memory();    // bytes by kind
```

`set_shadows(true)` and the other existing calls keep compiling and meaning what they do now (`impl From<bool> for Shadows`). Nothing public is removed in 0.1.x; anything that should move (per-mesh layers onto `Material`) is noted for 0.2, as ROADMAP already does.

## 8. Luau API

Everything is a prop on `ui.scene3d` or a child of it, each value a number, a state, a function or a `ui.tween`/`ui.spring` binding, written to the world in Rust with no script per frame.

```luau
ui.scene3d {
	w = "100%", h = "100%",
	antialiasing = "taa", render_scale = 0.67,
	post = { bloom = { intensity = 0.04 }, exposure = { auto = true, compensation = exposure },
	         grade = { lut = "assets/dawn.cube" }, vignette = 0.2, grain = 0.03 },
	shadows = { cascades = 4, softness = "pcss" },
	ambient_occlusion = true, reflections = true,
	ui.sky { sun = sun_direction, intensity = 20 },
	ui.fog { density = 0.02, height = 12 },
	ui.volume { shape = "box", size = { 60, 8, 40 }, density = mist },
	ui.light { kind = "spot", position = { 4, 9, 0 }, lumens = 4000, cone = { 30, 40 }, shadows = true },
	ui.probes { size = { 80, 20, 60 }, count = { 8, 4, 8 } },
	ui.mesh { geometry = "plane", size = { 200, 1, 200 },
	          material = ui.shader { src = "water.wgsl", params = { wind = wind, waves = waves } } },
	ui.particles { max = 200000, spawn = { shape = "box", size = { 80, 2, 80 }, rate = rain_rate },
	               lifetime = { 0.6, 0.9 }, velocity = { 0, -22, 0 }, forces = { "gravity", { wind = wind } },
	               collide = { depth = true, on_hit = splashes }, draw = "streak" },
	ui.compute { src = "fft.wgsl", buffers = { state = wave_state }, textures = { height = waves }, every = "frame" },
}
```

- **Types** go in `types/zgx.d.luau.in`; reference pages in `docs/reference/` (one each for post, lighting, materials and shaders, particles, compute, media and sky, probes).
- **Lint rules** (new): a 3D prop read once with `:get()`; a shader `param` driven by `ui.every` (use a binding or `time`); `ui.particles` inside `ui.each` over a large list; a `ui.compute` readback read in a per-frame callback; an emitter with a `max` above the tier B cap and no fallback.
- **Plugins:** shaders and compute passes from a sandboxed plugin count against the plugin's memory, as its meshes do, and are compiled off the main thread with a time limit.

## 9. Assets

- **Compressed textures:** KTX2 with BasisU (UASTC and ETC1S), transcoded at load to BC7 or ASTC by what the device has, to RGBA8 otherwise; BC and ASTC payloads loaded directly. Mip chains come from the file; nothing is generated at run time. Behind a `ktx2` feature, with the transcoder's size measured before it's a default anywhere.
- **Loading** off the main thread (the pattern models use today), with a **streaming budget**: `World::set_texture_budget(bytes)` loads low mips first and the rest by screen coverage, dropping the highest mips of what's least seen when over budget.
- **Hot reload** in debug builds for textures, models, shaders and LUTs, by the file watching zgx-script already has.
- **Offline tool:** a `zgx assets` subcommand (in the `zgx` CLI, behind a feature) that compresses textures, builds mips, LODs and meshlets, and bakes probes, so none of that runs in an app.

## 10. The demo

`examples/harbor` (location: [decision 12](#14-decisions)), Rust and Luau, public API only. Its geometry is procedural or modelled for it, its textures generated, its sounds synthesised or recorded for it; nothing is copied from a game. When it needs something the API can't express, it files an issue for the owning lane and waits; it doesn't work around it.

| In the scene | Built in the demo from | Engine primitives used |
| --- | --- | --- |
| Dawn sky, low sun through haze | Sun direction and sky settings on a timeline | Physical sky, sky as environment, cascades, exposure, grading LUT |
| Light shafts between cranes and stacks | Mist volumes placed along the quay | Froxel fog, density volumes, cascade shadows in fog |
| Water | A custom material: FFT heights from a compute pass, refraction and depth tint from the scene behind, foam from wave slope | Custom material (displace, surface, scene colour and depth), compute, SSR with probe fallback |
| Rain | An emitter of streaks, splash sub-emitters on depth hits, ripples written to a texture by compute | Particles, depth collision, events, compute, reactive mask |
| Wet ground, puddles | A world surface layer: darker, glossier, puddle mask from a noise and the surface's slope | Custom material as a layer, SSR, reflection probes |
| Containers by the thousand | Instanced meshes with per-instance colour and wear | Kept instances, culling, LODs, decals (markings, rust) |
| Cranes | glTF models, a few moving | Skins and animator, LODs, local shadows |
| Sodium lamps, deck lights, a welder's sparks | Spot and rect lights, a spark emitter with a point light | Clusters, area lights, local shadows, bloom, particles |
| Cabin windows, painted steel, tarps | Stock materials | Transmission, clearcoat, sheen, anisotropy |
| Bounce light under the gantries | Probe volumes over the quay | Irradiance probes, GTAO, contact shadows |
| Camera | A scripted fly-through and free orbit | TAA, upscaling, motion blur, depth of field |
| Sound | [design-audio.md](design-audio.md#the-demo) | Spatial emitters, reverb zones, occlusion |
| HUD and settings panel | zgx UI over the view | Proves UI stays sharp over TAA, and that every setting is a binding |

Targets: 60 fps at 1080p on a mid-range GPU and 120 fps on an M-series Mac. The demo grows a slice per milestone, and each slice is what that milestone's primitives are judged with.

## 11. Budgets

These are targets to be measured against, set before measuring; where a measurement shows one is wrong it's changed in this table with the number that showed it. GPU time is per frame with the feature at its default quality, read from timestamp queries where the backend has them and from A/B frame times (interleaved, repeated, with a control arm) where it doesn't. "M" is an M5 Max; the M1 Max must hold 60 fps on the demo. "RTX" is the 4090 in WING-PC. **There's no mid-range GPU in the fleet** ([decision 8](#14-decisions)).

| Primitive | 1080p M | 1080p RTX | 4K M | 4K RTX |
| --- | --- | --- | --- | --- |
| Depth, normal, velocity prepass (demo scene) | 0.5 | 0.3 | 1.2 | 0.6 |
| Sun cascades (amortised; one cascade redrawn a frame) | 0.8 | 0.4 | 0.8 | 0.4 |
| Local shadows (8 tiles redrawn) | 0.5 | 0.3 | 0.5 | 0.3 |
| Opaque forward, clustered, 256 lights | 2.0 | 1.0 | 6.0 | 2.5 |
| GTAO | 0.5 | 0.25 | 1.6 | 0.7 |
| Contact shadows | 0.3 | 0.15 | 1.0 | 0.4 |
| SSR | 0.8 | 0.4 | 2.6 | 1.2 |
| Sky LUTs (amortised) + froxel fog | 0.9 | 0.5 | 0.9 | 0.5 |
| Transparents + 200,000 particles (OIT) | 1.0 | 0.5 | 2.5 | 1.0 |
| TAA | 0.4 | 0.2 | 1.4 | 0.6 |
| Upscale instead of TAA (0.67 scale) | 0.6 | 0.3 | 1.6 | 0.7 |
| Bloom | 0.4 | 0.2 | 1.2 | 0.5 |
| Depth of field | 0.8 | 0.4 | 2.6 | 1.1 |
| Motion blur | 0.5 | 0.25 | 1.6 | 0.7 |
| Exposure, tone map, grade, vignette, grain, CA | 0.25 | 0.1 | 0.8 | 0.3 |
| Probes (4 updated a frame) | 0.5 | 0.3 | 0.5 | 0.3 |
| GPU culling, 100,000 instances | 0.2 | 0.1 | 0.2 | 0.1 |

Everything on is about 11 ms at 1080p on the M5 Max. The demo's 120 fps profile there (8.3 ms) renders at 0.67 scale and upscales, which brings the screen-space rows down by about half; depth of field and motion blur are for the fly-through at 60. 4K is reached by upscaling from 1440p, not natively.

**CPU:** the frame's CPU side at the demo's scene is budgeted at 1.5 ms on the M5 (cull, clusters, animation, uploads), counted by new gate cells ([section 12](#12-tests-gates-and-proof)).

**Memory** at 1080p, everything on: HDR colour 16.6 MB, history 16.6, depth 8.3, prepass normal and velocity 16.6, half-resolution set 10, bloom chain 5.5, froxels 15, scene-behind with mips 22, cascades (4 × 2048² at 16 bits) 32, local shadow atlas (4096² at 16 bits) 32, clusters and lists 2, sky LUTs 1, OIT 12: **about 190 MB of targets**, before the app's assets and the driver's own blocks. Each feature's bytes are in `World::memory()` and released when it's turned off.

*Measured, R1:* with the post chain on under MSAA 4x, the samples' colour is Rgba16Float (8 bytes a sample) where a world's background is a plain colour or transparent: alpha carries coverage so the background is composited after tone mapping, unexposed and unbloomed. That is 66 MB at 1080p against today's 33 (+33 MB; +133 MB at 4K), until TAA removes the multisampled colour. Worlds whose background is part of the scene's light (a sky, an HDR backdrop) draw it in Rg11b10Ufloat, 4 bytes a sample, no growth. Not done, the measured follow-up if a scene needs the memory: Rg11b10 for any opaque background with coverage taken from the stored MSAA depth instead of alpha. Its limit: translucent meshes write no depth, so over the background they would add light rather than cover it. Bloom's steps are Rg11b10 where renderable (2.8 MB at 1080p, not 5.5). On iOS the defaults halve the shadow atlases and froxel depth, and the render scale defaults to 0.67. A world with nothing on keeps today's memory exactly.

## 12. Tests, gates and proof

- **Reference images, part one (fresh against retained):** each primitive has scenes rendered two ways: by a renderer made fresh for that frame, and by one that reached the same state through a sequence of changes, resizes and feature toggles. They must match exactly on software adapters and within one level on hardware. This catches stale caches and history, the retained design's own risk.
- **Reference images, part two (goldens):** small PNGs (256² or smaller) checked in under `crates/zgx-3d/tests/reference/`, compared per backend with a stated tolerance per image (max channel difference and the fraction of pixels allowed to differ), blessed with `ZGX_BLESS=1`. The repository holds none today; this harness is the first thing built. Budget: 5 MB of images in the repository.
- **Reference images, part three (other renderers):** side by side with an established renderer at a stated tolerance, images and the scene files kept in `docs/images/3d-reference/`: the Khronos glTF sample viewer for materials, Blender Cycles (path traced, as ground truth) for lighting, shadows, sky, fog and GI, Filament for tone mapping and bloom. The metric is FLIP mean error plus a per-image note of every visible difference, stated in the PR. What counts as close enough is the maintainer's call per primitive.
- **Tests fail first:** each PR names the test that failed before it.
- **Temporal tests** run a fixed sequence of frame times and compare frame N, so TAA, particles, exposure and fog history are deterministic.
- **Gate cells (instructions, cachegrind):** new light cells on `world_bench`, each a small scene that runs one primitive's CPU path so its cost shows by function: `3d-post`, `3d-taa`, `3d-lights` (256 lights, clusters), `3d-cascades`, `3d-shader` (custom materials), `3d-compute`, `3d-particles`, `3d-cull` (100,000 instances, camera moving), `3d-anim`, `3d-probes`, `3d-media`. Each is added in the PR *before* the primitive lands, so the primitive's own PR shows its cost against a floor. `3d-textured` and `3d-mixed` must stay at or below main. Every UI cell stays within +0.02%, read numerically, cell by cell.
- **Reading the gate:** `gate.py` fails only above +1%, so its verdict line proves nothing here. Every merge note carries the rows read as numbers against +0.02% for UI cells, and against each 3D cell's recorded spread.
- **Zero-cost proof for UI apps:** UI gate cells; `inline_scopes.py` (zgx-3d's own hot functions are added to its list in R0); a new `scripts/perf/sizes.py` comparing the stripped size and symbol sizes of the gate's UI binaries and of `zgx-run` with `scene3d` off between two commits (nothing of ours may move); `footprint` at rest from outside the process; idle wakeups by `proc_pid_rusage`, as validation.md measures them today.
- **GPU cost:** a `harbor_bench` mode of the demo with a fixed camera path, reporting per-pass GPU time, frame-time percentiles and frames drawn; each arm proves it drew (frame count, a pixel hash that differs from the cleared target, no error scope output).
- **Idle and memory:** frames and wakeups over 10 s for a still world with every feature on (0 frames after settling); the open-and-close footprint loop of section 4.3.
- **Native probes:** a `scene3d_post` Linux probe (real X11 and Wayland windows, a world with the post chain, pixels read from the window) registered in `scripts/linux_smokes.py`; the same scene in the macOS and Windows window probes.
- **CI:** zgx-3d's GPU tests run on Linux's llvmpipe today; R0 adds them to the Windows job on WARP and the macOS job on Metal, with the job time measured before and after. What CI can't run (hardware Metal, the D3D11 view host on real drivers, iOS) is run on the fleet and named in each merge note.

## 13. Milestones

Each ends with a report to the maintainer: what landed with numbers, what was compared and how close, what the demo needed that wasn't there, what's open, what needs a decision. The order is the brief's. Audio milestones (A1–A3) are in design-audio.md and interleave as marked.

| | Milestone | Exit criteria |
| --- | --- | --- |
| R0 | **Foundations.** `render.rs` split into modules with no change in behaviour; the pass list with today's three passes; kept instance data and CPU frustum culling; the comparison harness of section 2a; `caps()`; the reference-image harness; the first new gate cells; zgx-3d's GPU tests in CI; the D3D11 view host. | Today's 57 pixel tests pass unchanged; `3d-textured` and `3d-mixed` at or below main on arm64 and x86; UI cells flat; a view draws on D3D11 with `d3d11_parity` cases. |
| R1 | **HDR and the post chain:** HDR targets, bloom, exposure (manual and automatic), grading and LUTs, vignette, grain, chromatic aberration, the `PostPass` interface, then depth of field and motion blur (which need R2's velocity; they land with it). **First cut, landed:** the HDR scene with a tone-weighted MSAA resolve, bloom, exposure (manual; automatic by GPU histogram on tier A), grading (white balance, lift/gamma/gain, saturation, display-space 3D LUTs), vignette, grain, chromatic aberration, `PostPass` (HDR and display stages). **Not in it, still owed:** `PostPass::after(Stock::…)` ordering among the stock passes (bloom is mixed in the last pass, not into the scene, so "after bloom" needs bloom applied to the scene first); automatic exposure on tier B (a mip-average of log luminance by fragment passes); depth of field and motion blur (with R2's velocity); the scene in Rg11b10 for worlds whose background is part of the light (with the sky); measured GPU costs against the budget table (with the perf lane's busy-time harness). | Goldens on every backend; Filament and Cycles comparisons; budgets met; idle after settling; a world with nothing on unchanged; Luau props, docs, example. |
| R2 | **TAA and upscaling:** prepass (depth, normal, velocity), TAA, reactive mask, upscaling. | The three no-ghosting scenes against supersampled references; UI over a view bit-identical with TAA on and off; settles in 8 frames. |
| A1 | Audio core, UI sounds first (design-audio.md). | |
| R3 | **Lighting and shadows:** clusters, physical units, area lights, cascades, local shadows, PCSS, contact shadows. | 256 lights within budget; cascade stability test (no shimmer under a slow camera pan: frame-to-frame difference below a threshold); Cycles comparisons. |
| R4 | **Custom materials and compute:** `MaterialShader`, surface layers, `Compute`, buffers, textures, indirect draws, readback. | The demo's water and wetness written on them; a shader error reports the app's line; stability promises documented; tier B behaviour tested. |
| A2 | Spatial audio. | |
| R5 | **Materials:** sheen, anisotropy, iridescence, emissive strength, subsurface, alpha to coverage, decals. | Khronos sample viewer comparisons for every extension's sample models. |
| R6 | **Media and sky:** height fog, froxel fog, density volumes, physical sky. | Cycles volume comparisons; sky against the reference model's tables; budgets. |
| R7 | **Particles.** | 200,000 rain particles within budget; soft, lit, shadowed, colliding; no TAA smear; tier B fallback; Luau. |
| R8 | **GI and reflections:** probes, reflection probes, SSR, GTAO, GPU environment prefilter. | Cycles comparisons of bounce light in a test room; SSR fallback has no seams at screen edges (a test). |
| R9 | **Geometry and assets:** kept instances, culling, LODs, heightfields, dynamic geometry, KTX2, streaming, hot reload, `zgx assets`; meshlets if needed. | 100,000 instances within the CPU and GPU budgets; a still camera uploads nothing; texture budget held under a fly-through. |
| R10 | **Animation:** animator, layers, additive, root motion, IK hook. | Poses against Blender's for the same clips and weights. |
| D | **The demo**, a slice per milestone, finished after R10. | 60 fps at 1080p mid-range, 120 fps on the M5 Max, public API only, every gap filed and closed. |

## 14. Decisions

Settled Nov 10 2026 by the coordinator, the maintainer having said designs need no approval and that the bar is the result. Each can be reopened by a measurement.

1. **Today's path is the pass list with nothing on,** not a second renderer. `3d-textured` and `3d-mixed` stay at or below main; today's pixels don't change.
2. **Each primitive sits behind a zgx-3d Cargo feature** (`post`, `lighting`, `shaders`, `compute`, `particles`, `media`, `gi`, `scale`, `ktx2`), measured in KiB on `zgx-run`'s `dist` build. None is a default of `zgx-run` or `zgx-video` until the maintainer chooses.
3. **The Direct3D 11 view host is built in R0,** by the lane that owns D3D11 parity. It's needed whatever is decided about Windows' default renderer.
4. **GL is the reduced tier** tabled in section 3.
5. **Probe GI has two update paths from the start:** ray-traced where the device has ray queries, rasterised elsewhere (2b).
6. **Particle transparency defaults to weighted blended OIT,** with a per-emitter sort. Side-by-side frames go to the maintainer before R7 merges; work doesn't wait on them.
7. **No display HDR in this plan.**
8. **The 4090 stands in for a mid-range GPU** at a 4 ms budget for the demo at 1080p until a 4060-class machine can be measured.
9. **Reference renderers:** the Khronos viewer for materials, Cycles for light transport, Filament for post, and Unreal Engine 5 for the overall look (internal comparison only, nothing published); FLIP as the metric; every visible difference stated. Looks are shown to the maintainer.
10. **Dependencies** under MIT, Apache-2.0, BSD or zlib are added on a size measurement; anything else goes to the maintainer first.
11. **Stability promises** are section 5's: versioned WGSL contracts, additions only within a version.
12. **The demo lives in this repository,** `examples/harbor`, as a crate outside the workspace's default members that depends on the zgx crates by their public API only. Its assets are generated by code or scripts in it.
13. **Surface layers** (5.1) are in the engine.
14. **iOS** work goes on the iOS host in draft (#1591, #1592 and the PRs stacked with them) once it lands; until then zgx-3d is kept compiling for `aarch64-apple-ios` and no primitive is called done on iOS.

## 15. Lanes and who owns what

`render.rs` is one 129 KB file, so nothing can be built in parallel until R0 splits it. The render-graph lane does R0 alone; other rendering lanes start on their own modules only after it merges.

| Lane | Owns | Starts |
| --- | --- | --- |
| Render graph and post | `render/graph`, `render/targets`, `render/post/*`, `temporal` | R0 now (after approval), then R1, R2 |
| Perf and validation | Reference harness, gate cells, `harbor_bench`, CI, probes; reads every PR's rows | R0, alongside |
| Platform parity | D3D11 view host, GL tier, iOS, `caps()` in zgx-gpu's seam | R0, alongside (its files are in zgx-gpu) |
| Lighting and shadows | `render/lights`, `render/shadows`, `render/clusters` | R3 |
| Materials and shaders | `render/shader/*`, `material.rs`, the custom material interface | R4, then R5 |
| Compute and GPU data | `compute.rs`, `buffer.rs` | R4, after the shader interface's parameter types merge (it goes first) |
| Media and sky | `render/media`, `render/sky` | R6 |
| Particles | `particles/*` | R7 |
| GI and reflections | `render/probes`, `render/ssr`, `render/gtao`, `environment.rs` | R8 |
| Geometry and assets | `geometry.rs`, `render/cull`, `render/instances`, `assets/*`, `gltf.rs` | R9 (kept instances may start earlier if the demo needs it) |
| Animation | `animation.rs`, `animator.rs` | R10 |
| Audio core, spatial audio | `crates/zgx-audio` | A1 now (no shared files), A2 after R3 |
| Demo | `examples/harbor` only; files issues, never edits the engine | With R1 |

Shared files (`world.rs`, `lib.rs`, the Luau bindings and `zgx.d.luau.in`, `docs/reference/`) are edited by one lane at a time, in milestone order; a lane needing one out of turn asks the coordinator, who says who goes first.
