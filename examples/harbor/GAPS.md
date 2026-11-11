# What the harbor needed that the public API can't express

Each entry is a primitive any game would need, in general form, with the smallest API that would close it. Something awkward to express counts as well as something impossible. None is worked around in the demo: where one bites, the scene shows the flaw and this file says so. Entries go to the 3D work's coordinator as they're found, who routes each to its owner.

**States:** open (nothing covers it yet), planned (the design has it; the demo waits for its milestone), partly closed, closed (landed; the demo uses it).

| | What | Owner | State |
| --- | --- | --- | --- |
| G1 | A world's frame statistics are hidden | graph lane | open (design §7) |
| G2 | The environment can't be the background; the background ignores exposure | GI lane | open |
| G3 | Per-instance data beyond colour and emission | materials lane | closed (7c6595faa): the containers' wear |
| G4 | No per-mesh opt-out of the sun's shadow | lighting lane | partly closed by cascades |
| G5 | The offscreen renderer has no wait but a readback | zgx-gpu (another team) | open |
| G6 | Animators play glTF clips only: nothing made in code can be animated by one | animation lane | open |
| G7 | A GPU view's frame time is the wall clock's, even under a `VirtualClock` | zgx-gpu | closed (73933e56b) |
| G8 | An emitter's rate, velocity and forces can't change once it's added | compute lane | open |
| G9 | A 3D emitter can't start partway into its sound or take its own pitch | audio lane | open |
| G10 | `scene_color` is the scene as displayed, not its light: a material can't put it back | materials lane | answered (bb24bbe5b): `scene_is_light()`; the water uses it |
| G11 | Nothing says when a surface layer has taken effect | materials lane | closed: `shader_ready` answers for layers |
| G12 | The exposure automatic exposure has adapted to can't be read | graph lane (post) | closed (9161837ea): `World::adapted_exposure` |
| G13 | A bus's level can't be read | audio lane | closed (8df625dcb): `Bus::set_metered`, `Bus::level` |
| P1 | GPU particles aren't reproducible run to run, under a virtual clock | particles lane | open (bug) |
| L1 | Spot lights are binned by their sphere, not their cone | lighting lane | closed (dedd5f83f); cost re-measured |
| L2 | A light at zero intensity is still binned and shaded | lighting lane | open (performance) |
| L3 | Frames aren't reproducible run to run with cascades or with clustered lights | this M1 Max's GPU at MSAA edges | M1 Max only, under investigation |
| L4 | One moving caster redraws every caster in its cascades | lighting lane | open (performance) |
| L6 | One moving mesh restarts every probe's rounds | GI lane | open (performance) |
| L5 | Sound occlusion tests a large mesh's triangles one by one, spending the frame's budget | audio lane | closed (6a8e9772a) |
| M2 | The post chain's memory: 108 MB measured against the docs' 33 MB at 1080p | graph lane (post) | measurement |
| M1 | Anything moving costs about 1.3 M instructions a frame over a camera-only frame | graph lane | measurement |
| B1 | Short checkbox and radio labels lost their last glyph in the bench | the bench's setup | resolved by setup; engine default landing on main |

## G1. A world's frame statistics are hidden

**Needed by:** any app with a performance HUD, any benchmark, any test of "a still world draws nothing".

`World::frames_drawn`, `culled` and `set_culling` exist but are `#[doc(hidden)]`, as is `World::viewport()`. (`World::lights_left_out` is public and the HUD uses it.) `harbor_bench` counts "frames with the world changed" by `World::revision()`, which is inferred, not observed.

```rust
pub struct Stats { pub frames: u64, pub draws: u32, pub instances: u32, pub culled: u32, pub triangles: u64 }
impl World { pub fn stats(&self) -> Stats; }
```

## G2. The environment can't be drawn as the background, and the background ignores exposure

**Needed by:** any game with an HDR sky or skybox, and any scene whose light changes (day to night).

**The environment is never seen.** `set_background` takes one flat sRGB colour, so the environment lights and reflects but never shows. The harbor's background is a flat lilac.

**The background ignores exposure.** It's a display colour, outside exposure: lit for night (`SUN=-12 EXPOSURE=8`), the scene goes dark while the background stays dawn-bright.

```rust
pub enum Background { None, Color([f32; 4]) /* linear, exposed */, Environment { blur: f32 } }
impl World { pub fn set_background_of(&self, background: Background); }
```

## G3. Per-instance data beyond colour and emission

**Needed by:** any instanced crowd: wear, dirt, damage, tints, animation offsets.

The containers vary in hue; their wear can't vary per box. It's covered by §5.1's four instance floats.

## G4. A mesh can't opt out of the sun's shadow

**Needed by:** any outdoor scene with a ground plane, water, far terrain or a backdrop.

**Stage 0 (one map), measured.** The map was fitted to every opaque mesh, so the 6 km water plane and the far ridge spread it over kilometres. Shadows changed 4.6% of pixels at 40 s, against 17.6% with the water cut to 700 m and the ridge gone.

**With cascades (stage 1), measured.** At 40 s shadows change 18.6% of pixels, against 4.7% with one map. The resolution is fixed.

**Still open: the opt-out.**
- A ground, water or sky mesh still casts into every cascade it crosses.
- A backdrop's shadow is wrong in kind.

```rust
impl World {
    pub fn set_casts_shadows(&self, id: ObjectId, on: bool);    // default true
    pub fn set_receives_shadows(&self, id: ObjectId, on: bool); // default true
}
```

## G5. The offscreen renderer has no wait but a readback

**Needed by:** offscreen benches, video renderers.

`GpuRenderer` submits and returns. The only public wait is a readback, so the harbor's frame times include one, measured apart (0.7–1.3 ms at 1080p, M1 Max) and printed beside them.

```rust
impl GpuRenderer { pub fn finish(&self) -> Result<(), GpuError>; }
```

## G6. Animators play glTF clips only

**Needed by:** any game animating what it builds in code: procedural machines, doors, platforms, cranes, UI-driven rigs.

**What exists.** `World::add_animator` takes a `Placed`, which only `place_model` makes from a glTF `Model`. `Clip` has no constructor: its keys come from a glTF file. And all of it is behind the `gltf` feature.

**What that leaves.** Objects built with `Object::mesh` and groups can't be given clips, layers, crossfades, events or root motion. An app moves them by hand each frame, and that's a CPU loop outside the animator, so the world doesn't know it's animating.

The harbor's gantry is modelled for the demo in code. To give it an animator it's written as a `.glb` by the demo itself (`glb.rs`, `crane.rs`), read back with `Model::from_bytes`, and placed four times, each with an animator playing its work.

That's the design's path for the cranes (§10: glTF models "modelled for" the demo), not a workaround. What it cost is the general need:
- **A glTF writer:** about 300 lines.
- **Unused decoders:** the `gltf` feature, which brings PNG and JPEG decoding the demo doesn't use.
- **Nodes by index only:** reaching the trolley and the spreader goes through `Placed::node(index)`, with the indices kept by the writer. A model's nodes have names, but nothing finds one by name.

```rust
impl Placed { pub fn node_named(&self, name: &str) -> Option<ObjectId>; }
```

```rust
let clip = Clip::new(4.0)
    .translation(0, &[(0.0, [0., 0., -26.]), (2.0, [0., 0., 10.])])   // channel on target 0
    .rotation(1, &[(0.0, Quat::IDENTITY), (4.0, Quat::from_euler(0., 1., 0.))]);
let animator = world.add_animator_for(&[trolley, spreader]);          // targets, in order
world.play(animator, &clip, Play::new())?;
```

## G7. A GPU view's frame time is the wall clock's, even under a `VirtualClock`: closed

**Closed by 73933e56b** (one clock per window frame): `ViewportFrame::time` is now `zgx::clock::now()`. Measured from the harbor: working cranes on their animators, under the bench's `VirtualClock`, give the same 600 frames run to run (0 differ; before, 600 of 600). Rain still doesn't repeat: P1.

(The harbor's own bench had a bug here too, found by the animation lane's copy of it: `VirtualClock::advance` takes the time since the clock's start, and the bench passed the step, so its clock stood at 0.1 s. Fixed. G7's evidence stands: with the clock standing still, frames that differed could only have followed the wall clock.)

### As first found

**Needed by:** video renders, benchmarks, reference images, any offline render of moving 3D; design §4.4 promises it ("time comes from the frame, never the wall clock, so zgx-video's virtual clock renders a world deterministically").

`zgx::clock::VirtualClock` makes `zgx::clock::now()` the clock's time on its thread. But zgx-gpu stamps each GPU view's frame with `std::time::Instant::now()` (`viewport.rs`, the `ViewportFrame` a renderer hands a painter). A world reads its particles' and animators' time from that stamp.

So a 3D world drawn under a virtual clock moves by the wall clock. That includes zgx-video's `Stage`, which installs one.

**Measured:** `harbor_bench` installs a `VirtualClock` and advances it a fixed step a frame. With rain, two runs of the same binary differ in all 600 of 600 frames, the last frame included. Without rain, the stage-0 scene repeated exactly.

A world's own way to be given a time, `World::animate(time)`, is `#[doc(hidden)]` and covers animators alone.

**Smallest change:** the view frame's `time` from `zgx::clock::now()`. An app drawing a world itself would need a public `World::advance_to(time)` for particles and animators alike.

## G13. A bus's level can't be read

**Needed by:** any mixer view, any game ducking music under dialogue by level, and any test proving a mix ran.

zgx-audio keeps an output-peak meter of its own, but nothing public reads a bus's level, or master's. The harbor's bench therefore measures master's RMS and peak from the offline mixer's samples itself (−21.7 dBFS RMS, −9.3 dBFS peak), and can't say what each bus (world, ui, its reverb) contributed.

```rust
impl Bus { pub fn level(&self) -> Level; } // RMS and peak since last asked, per channel
```

## P1. GPU particles aren't reproducible run to run, under a virtual clock

**Needed by:** video renders and reference images of anything with particles (design §4.4: "particles, noise and TAA jitter are functions of frame time and a frame index").

With G7 closed, the harbor's frames repeat run to run without rain (0–3 of 600 differ, within L3's edge noise), but not with it: 427–444 of 600 frames differ. Those frames differ in 4–439 pixels by up to 170 levels, so drops are in different places or blended in a different order.

Arm: `POST=0 PROBES=0 CASCADES=0 LAMP_REACH=0 SOUND=0 CRANES=still WATER=stock`, with and without `RAIN=0`, twice each with `HASHES=`.

The particles' own randomness (or the order a compute pass hands out slots in, which alpha blending shows) depends on something besides the frame's time and index.

## G8. An emitter's rate, velocity and forces can't change once it's added

**Needed by:** any weather, fire or effect whose strength changes (rain easing off, wind rising, a jet throttling).

After `add_emitter`, only `set_emitting` (on or off), `burst` and `move_emitter` change one. To vary rain or wind, an app removes the emitter and adds a new one, and a new pool loses the drops in flight. The harbor's panel has "Rain" on and off, not an intensity, and its wind can't follow a wind setting.

```rust
impl World {
    pub fn set_emitter_rate(&self, id: EmitterId, per_second: f32);
    pub fn set_emitter_forces(&self, id: EmitterId, forces: &[Force]);   // or update_emitter(id, |e| ...)
}
```

## G9. A 3D emitter can't start partway into its sound or take its own pitch

**Needed by:** any game with several of the same machine, creature or crowd: engines, generators, fans, birds.

`Emitter3d` has `looping`, `gain`, `falloff`, `cone`, `doppler`, `occlusion` and `air`. Nothing says where in the sound it starts, and nothing sets a pitch of its own (Doppler moves pitch, nothing else).

The harbor's four cranes play one synthesised drive, so the four loops start together and stay in phase, and the hum combs as the listener moves between them. A plain voice has `Play::pitch`; an emitter doesn't. The only way to vary them is to make each crane a sound of its own.

```rust
impl Emitter3d { pub fn from(self, seconds: f32) -> Self; pub fn pitch(self, pitch: f32) -> Self; }
```

## G10. `scene_color` is the scene as displayed, not its light: answered

**Answered by bb24bbe5b:** with the post chain on, `scene_color` is already the scene's light before exposure and tone mapping; with it off, it's the scene as displayed; and `scene_is_light()` tells a material which. The harbor's water now leaves it as it is when it's light, and undoes the panel's exposure only when it's the displayed scene.

**What the harbor got wrong before, and what it showed:**
- The water multiplied the scene's light by 2^11 more, assuming the displayed scene. That gave the turquoise (the flat background, the only thing under most of the water, made bright) and a white band where the water meets the hull.
- Corrected, the water reads dark slate-blue with the sky reflected toward the horizon: `~/zgx-aaa/demo-frames/stage1-sea/`.
- The background still sits outside the scene's light (G2), so where it's seen through the water it's dimmer than the sky would be.

### As first found

**Needed by:** any refracting or transmitting material: water, ice, thick glass, heat haze.

`scene_color(uv, lod)` returns the opaque scene *as displayed*: exposed, tone mapped (ACES here), then decoded back to linear. A material wanting the light coming through it has to undo both. The exposure it can undo if the app hands it in: the harbor's water takes `unexpose`, which the panel's exposure keeps current. The tone curve it can't. So refraction through the harbor's water is the tone-mapped scene scaled back, brighter and flatter than the scene's light.

It's the design's known gap (§1: no HDR target), and closes with R1's HDR target. The interface would then promise scene-linear light, before exposure, as §5.2 promises for HDR post passes.

**Seen with G2:** the water sees mostly the flat background (nothing else is under it, and there's no `scene_depth` to tint by depth). That background is a display colour outside exposure, so un-exposed it comes through far too bright: the harbor's water reads turquoise. That's these gaps showing, not a look chosen.

## G11. Nothing says when a surface layer has taken effect

**Needed by:** any app waiting to show a change without a pop (a weather change, a first frame for a thumbnail).

`World::shader_ready(id)` says when a mesh shader's pipelines are in. For a surface layer it stays false (here after 630 frames) although the layer draws: `WET=0` against wet changes 7–56% of the frame's pixels. A layer compiles into every lit mesh's pipelines, so its readiness is theirs, and nothing reports it.

```rust
impl World { pub fn layer_ready(&self, layer: ShaderId) -> bool; } // or shader_ready answering for layers too
```

## G12. The exposure automatic exposure has adapted to can't be read

**Needed by:** anything that must know what the frame's exposure is: a material reading `scene_color` (G10), a HUD showing exposure, a camera model, a sound or effect keyed to brightness.

With `Post::auto_exposure` the frame's exposure is the GPU's own, eased in frame time, and nothing returns it. The harbor's water undoes the exposure on `scene_color` through a parameter. With adapting exposure on, it can only assume EV 11.

By night (`SUN=-12`) the exposure adapts about nine stops brighter, and the water, multiplying by the wrong factor, blows out to white-cyan (`~/zgx-aaa/demo-frames/stage1-post-night/`). With G10 closed (`scene_color` in the scene's light) the water wouldn't need it. A readback would serve the rest:

```rust
impl World { pub fn exposure_now(&self) -> Option<f32>; } // stops, a frame or so late
```

## L1. Spot lights are binned by their sphere, not their cone: closed

**Closed by dedd5f83f** (spots binned by their cones). Re-measured on it (600 frames, `POST=0 PROBES=0 RAIN=0 SOUND=0 CRANES=still WATER=stock`, GPU busy ms a frame). `LAMP_REACH` now scales each lamp's physical reach (to 0.5 lux: 71 m for a mast flood); 1.7 is about the first sweep's hand-set ranges:

| LAMP_REACH | 0 | 0.25 | 0.5 | 1 | 1.7 |
| --- | --- | --- | --- | --- | --- |
| GPU busy ms/frame | 5.53–5.57 | 5.88 | 6.89 | 8.81–9.51 | 13.07–13.78 |
| Minstr/frame | 9.2 | 9.9 | 10.0 | 10.6 | 11.6–11.8 |

So at the first sweep's ranges the lamps cost +7.5 to +8.2 ms, where before cone binning they cost +13.2 ms: about 40% less. At their physical reach, +3.3 to +4.0 ms. They're still the frame's largest cost.

### As first found

**Needed by:** any scene of floodlights, street lights or stage lights, aimed down or narrow.

`render/clusters.rs` lists a light in every cluster its *sphere* (position, range) touches. So a 35 m mast flood aimed down is listed in clusters above it and outside its cone.

Measured on the harbor (221 lights, 176 of them downward spots, 1080p, M1 Max, GPU busy a frame), scaling every lamp's range by `LAMP_REACH`:

| LAMP_REACH | 0 (no lamps) | 0.25 | 0.5 | 1 (the original 70–120 m ranges) |
| --- | --- | --- | --- | --- |
| GPU busy ms/frame | 3.58 | 4.68 | 7.82 | 16.8 |

Cost follows lights per cluster, which a cone test would cut for every aimed light. The harbor now gives each lamp the reach where it falls to 0.5 lux (`scene::reach`): 71 m for a 32,000 lm flood. Its lamps then cost 4.4 ms.

**Smallest change:** bin a spot by a bounding cone (or the sphere capped by the cone's plane), not the full sphere.

## L2. A light at zero intensity is still binned and shaded

**Needed by:** any game switching lights off (lamps at day, a blackout, a flicker).

The settings panel's "Lamps" turns 221 lights to zero lumens. With the original ranges, GPU busy stays at 16.5 ms, against 16.8 lit (`LAMPS=0`). `render/lights.rs`'s `gather` packs every point and spot light whatever its intensity, so an app has to remove its lights to save their cost and add them back to turn them on.

**Smallest change:** leave lights with no intensity out of the gathered list.

## L3. Frames aren't reproducible run to run with cascades or with clustered lights

**On this M1 Max only, under investigation** (the coordinator's note): a diagnostic showed this machine's GPU drawing one of two images at MSAA edges, with byte-identical shaders. A frames' hash that differs here isn't taken as the demo's bug, or the engine's, unless it differs beyond edges.

What the harbor saw:
- **How big:** up to 257 scattered pixels by up to 5 levels, on container faces and edges.
- **When:** in a few frames of 600, with cascades or with clustered lights on.
- **Repro:** `for i in 1 2 3; do HASHES=h$i.txt harbor_bench; done; diff h1.txt h2.txt`.

## L4. One moving caster redraws every caster in its cascades

**Needed by:** any scene with a few moving things in a large still one: cranes, vehicles, doors among buildings.

A cascade's map is drawn again when any caster inside it moves, and drawn whole. Four working cranes among 5,000 still containers redraw the cascades they're in every frame.

Measured (`CRANES=working` against `CRANES=still`, no rain, 600 frames, M1 Max):
- shadows off: +3.1 M instructions a frame;
- shadows on: +4.05 M and +0.7 ms of GPU.

So about 0.9 M instructions and most of the GPU time are the cascades' redraws.

**What would close it:** keep still casters' depth apart from moving ones' (a static layer redrawn when the cascade or the sun moves, the dynamic casters drawn over a copy of it each frame), as the larger engines do.

## L6. One moving mesh restarts every probe's rounds

**Needed by:** any probe-lit world with something moving in it: vehicles, doors, cranes, characters that rays see.

Per `docs/3d.md` a volume updates until every probe has had 16 rounds "since what rays see … last changed", and a traced mesh moving is such a change. The harbor has 5,184 probes in two volumes, and at the default 64 probes a frame a round is 81 frames, so settling takes about 1,300 frames.

Its four cranes move every frame, so the probes never settle:

| | Minstr/frame | GPU busy ms/frame |
| --- | --- | --- |
| cranes working, probes on | 44.8 | 15.9 |
| cranes working, `PROBES=0` | 31.0 | 12.2 |
| cranes still, no rain, probes on | 27.1 | 13.5 |
| cranes still, no rain, `PROBES=0` | 23.8 | 10.9 |

With the cranes working, probes cost 13.8 M instructions, 3.7 ms of GPU and about 97 MB. A crane on the quay changes what the probes near it see, not those 300 m away in the yard.

**What would close it:** restart the rounds of the probes whose rays could meet what moved (a box test against the volume's grid), not all of them.

## L5. Sound occlusion tests a large mesh's triangles one by one

**Needed by:** any world with sound and a large mesh: terrain, a ground, a heightfield, water.

Occlusion's rays go through a tree of meshes' boxes, then test each candidate mesh's triangles in turn, up to 100,000 a frame (rays past that wait). The harbor's near water grid is one mesh of 320,000 triangles spanning the harbor, so every ray from the listener to an emitter enters its box and spends the frame's budget on it.

Measured, 600 frames, everything on:

| | Minstr/frame |
| --- | --- |
| water grid among the blockers | 43.1 |
| the stock water (no grid) | 22.2 |
| sound off, with the grid | 16.34 |
| sound off, stock water | 16.13 |

So about 21 M instructions a frame went to the grid's triangles.

The harbor sets the sea's sound transmission to 1 (`set_sound_transmission`), as the API intends for what blocks nothing: water under every path. The cost then goes (22.6 M). A terrain that *does* occlude has no such way out.

**What would close it:** a tree over each large mesh's triangles (or meshes split into clusters for rays), so a ray tests the few triangles near it.

## M2. The post chain's memory

**A measurement, for the graph lane.** `docs/3d.md` says the chain costs "about 33 MB more than without the chain at 1920 by 1080". The harbor (1920×1080, 4x MSAA, bloom and automatic exposure) measures the process's physical footprint at 547 MB with it and 439 MB without (`POST=0`): 108 MB, repeated across runs. The footprint holds more than the targets (driver blocks, pools), so the gap may not all be the chain's, but it's three times the stated figure.

| | Minstr/frame | GPU busy ms/frame | footprint MB |
| --- | --- | --- | --- |
| chain on (bloom, auto exposure) | 24.7 | 10.41 | 547 |
| bloom, exposure fixed (`AUTO=0`) | 24.1 | 10.24 | 538 |
| no chain (`POST=0`) | 21.9 | 10.00 | 439 |

## M1. Anything moving costs about 1.3 M instructions a frame over a camera-only frame

**A measurement, for the graph lane.** Shadows off, no rain:

| | Minstr/frame |
| --- | --- |
| cranes still (`CRANES=still`) | 9.46 (1 crane), 9.65 (4) |
| idle animators (`CRANES=idle`, 4) | about the same as still |
| 1 crane working | 11.22 |
| 4 cranes working | 12.79 |

So about 1.3 M a frame is a fixed cost once any object moves, in a world of 5,717 objects, and about 0.45 M is each animator's own (6 nodes, 18 channels). Kept instance data with dirty ranges (design §6.7) is the place this would show if it isn't there yet.

## B1. Short labels lost their last glyph in the bench: resolved by setup

"Lamps" drew as "Lamp", "HUD" as "HU", "ACES" as "ACE" and "100%" as "100", in `harbor_bench`'s frames and in the repro on `lane/aaa-demo-repro-b1`.

**The cause, found by the engine's text lane, was the bench's setup.** Both made a `GpuRenderer` and mounted a `Ui` without `gpu.install_text(&mut ui.scene.borrow_mut())`. Without it, layout measures text by zgx's estimate (0.5 em a character) while the renderer draws real glyphs, so text sized by its own width was laid out too narrow and clipped. The window (`harbor`) installs it through zgx-desktop and was never affected.

**Now:** `harbor_bench` calls `install_text`, then `Ui::refresh_text_geometry`, right after making its renderer. Every label draws whole, and the repro exits 0 (`INSTALL_TEXT=0` shows the old clipping).

The text lane is also landing a fix on main that makes a renderer's text services the default for a scene with none installed. The harbor keeps the explicit call either way.

**Every bench number before this measured the estimate's text layout, not real text.** Old against new, same tip, same session (1080p @2x, 600 frames):

| | Minstr/frame, estimated text | Minstr/frame, real text |
| --- | --- | --- |
| with rain | 11.99 | 12.27 |
| no rain (`RAIN=0`) | 10.90 | 10.83 |

The frames' hashes changed with the HUD's text. The figures in the stage 0 and stage 1 reports stand as measured, with that difference in their text.

## Waiting on planned primitives (not gaps)

| In the scene | Lacking | Milestone |
| --- | --- | --- |
| Haze and depth: the far ridge reads as a dark wall | Height and distance fog, aerial perspective | R6 |
| A sky that follows the sun | The physical sky; the environment's prefilter on the GPU (a new sun costs 30–70 ms of CPU to relight, so the panel's sun slider moves the light, not the sky) | R6, R8 |
| Rain landing on the water, the containers and the cranes | Collision with the depth buffer (`Collide::Depth`, design §5.4); today one plane, so drops over the water stop at the quay's height, 3 m above it | R7 |
| Rain hidden behind what's in front of it | Particles drawn against the scene's depth: today they draw over every mesh (`docs/3d.md`, Particles) | R7, the pass list |
| Rain that takes the lamps' and the sky's light | Lit particles: today they're unlit, so the rain's colour is set by hand on the harbor's physical scale | R7 |
| Water's depth: shallows clear, deep water dark, the hull fading into it | `scene_depth` (with the depth prepass) | R2 |
| Waves from a spectrum (FFT) | A compute pass writing a texture the water reads; the water now sums six waves in its own shader | R4's compute feeding materials |
| Light shafts between the cranes | Froxel fog, density volumes | R6 |
| Ambient occlusion in the stacks' corners | GTAO | R8 |

## Closed

- **Lamps by the hundred:** 221 point and spot lights in lumens through clusters (stage 1). There were 8 nearest at stage 0.
- **Shadow detail over the quay:** four cascades over 600 m (stage 1).
- **Bounce light (probes):** two volumes, 5,184 probes, under the gantries over the apron and down the yard's canyons. Traced on the M1 Max (`ProbesStatus::Traced`).
- **Dawn to night, and glow (the post chain):** automatic exposure follows the harbor's physical light from a hazy dawn to a lamp-lit night (`SUN=-12`), and bloom spreads the lamp heads and windows.
- **Water and wet (custom materials):**
  - the sea is a material of the demo's own (`shaders/water.wgsl`): six wind waves moving a 1.6 km grid on frame time, ripples and rain rings in its normal, the scene behind seen through it;
  - a surface layer (`shaders/wet.wgsl`) darkens and smooths everything while it rains, with puddles on the ground.
- **Sound placed on emitters, with reverb zones:**
  - each crane's drive on its trolley, moving with the animator;
  - the ship's generator at its funnel;
  - a bell on a buoy;
  - rain as a bed;
  - reverb under each gantry's portal and in the container canyons;
  - all synthesised, mixed offline (no device can be opened).
- **Rain:** 45,000 streaks a second in a 160 m box following the camera, splashing where they land through a sub-emitter (stage 1).
