# What the harbor needed that the public API can't express

Each entry is a primitive any game would need, in general form, with the smallest API that would close it. Something awkward to express counts as well as something impossible. None is worked around in the demo: where one bites, the scene shows the flaw and this file says so. Entries go to the 3D work's coordinator as they're found, who routes each to its owner.

**States:** open (nothing covers it yet), planned (the design has it; the demo waits for its milestone), partly closed, closed (landed; the demo uses it).

| | What | Owner | State |
| --- | --- | --- | --- |
| G1 | A world's frame statistics are hidden | graph lane | open (design §7) |
| G2 | The environment can't be the background; the background ignores exposure | GI lane | open |
| G3 | Per-instance data beyond colour and emission | materials lane | planned (§5.1, landing) |
| G4 | No per-mesh opt-out of the sun's shadow | lighting lane | partly closed by cascades |
| G5 | The offscreen renderer has no wait but a readback | zgx-gpu (another team) | open |
| G6 | Animators play glTF clips only: nothing made in code can be animated by one | animation lane | open |
| G7 | A GPU view's frame time is the wall clock's, even under a `VirtualClock` | zgx-gpu's view seam | open (bug) |
| G8 | An emitter's rate, velocity and forces can't change once it's added | particles lane | open |
| L1 | Spot lights are binned by their sphere, not their cone | lighting lane | open (performance) |
| L2 | A light at zero intensity is still binned and shaded | lighting lane | open (performance) |
| L3 | Frames aren't reproducible run to run with cascades or with clustered lights | lighting lane | open (bug) |
| B1 | Short checkbox and radio labels lose their last glyph | zgx core (another coordinator) | open (bug); repro on `lane/aaa-demo-repro-b1` |

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

The harbor's gantries are code-built groups with a trolley and a spreader ready to move. The design (§10) has the cranes as glTF models "modelled for" the demo, so the harbor will generate a crane `.glb` in code to use the animator. That's the design's path, not a workaround; this entry is the general need.

```rust
let clip = Clip::new(4.0)
    .translation(0, &[(0.0, [0., 0., -26.]), (2.0, [0., 0., 10.])])   // channel on target 0
    .rotation(1, &[(0.0, Quat::IDENTITY), (4.0, Quat::from_euler(0., 1., 0.))]);
let animator = world.add_animator_for(&[trolley, spreader]);          // targets, in order
world.play(animator, &clip, Play::new())?;
```

## G7. A GPU view's frame time is the wall clock's, even under a `VirtualClock`

**Needed by:** video renders, benchmarks, reference images, any offline render of moving 3D; design §4.4 promises it ("time comes from the frame, never the wall clock, so zgx-video's virtual clock renders a world deterministically").

`zgx::clock::VirtualClock` makes `zgx::clock::now()` the clock's time on its thread. But zgx-gpu stamps each GPU view's frame with `std::time::Instant::now()` (`viewport.rs`, the `ViewportFrame` a renderer hands a painter). A world reads its particles' and animators' time from that stamp.

So a 3D world drawn under a virtual clock moves by the wall clock. That includes zgx-video's `Stage`, which installs one.

**Measured:** `harbor_bench` installs a `VirtualClock` and advances it a fixed step a frame. With rain, two runs of the same binary differ in all 600 of 600 frames, the last frame included. Without rain, the stage-0 scene repeated exactly.

A world's own way to be given a time, `World::animate(time)`, is `#[doc(hidden)]` and covers animators alone.

**Smallest change:** the view frame's `time` from `zgx::clock::now()`. An app drawing a world itself would need a public `World::advance_to(time)` for particles and animators alike.

## G8. An emitter's rate, velocity and forces can't change once it's added

**Needed by:** any weather, fire or effect whose strength changes (rain easing off, wind rising, a jet throttling).

After `add_emitter`, only `set_emitting` (on or off), `burst` and `move_emitter` change one. To vary rain or wind, an app removes the emitter and adds a new one, and a new pool loses the drops in flight. The harbor's panel has "Rain" on and off, not an intensity, and its wind can't follow a wind setting.

```rust
impl World {
    pub fn set_emitter_rate(&self, id: EmitterId, per_second: f32);
    pub fn set_emitter_forces(&self, id: EmitterId, forces: &[Force]);   // or update_emitter(id, |e| ...)
}
```

## L1. Spot lights are binned by their sphere, not their cone

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

**Needed by:** reference images, video renders, the temporal tests of design §12.

The same `harbor_bench` binary, run three or four times, gives frames that differ in a few frames of 600:
- **How big:** up to 257 scattered pixels, by up to 5 levels, on lit geometry: container faces and edges, across the whole frame.
- **Which frames:** different ones each run, mostly over the yard (frames 265–347 and 493).

| Arm (environment of `harbor_bench`) | Reproducible over 3–4 runs |
| --- | --- |
| `CASCADES=0 LAMP_REACH=0` (neither) | yes, every frame |
| `LAMP_REACH=0` (cascades, no lamps) | no |
| `CASCADES=0` (clustered lamps, one map) | no |
| default (both) | no |
| stage 0, before the `lighting` feature | yes (4 runs, and across rebases) |

Something in the cascades' or the clusters' path depends on timing, not only on the frame. **Repro:** `for i in 1 2 3; do HASHES=h$i.txt harbor_bench; done; diff h1.txt h2.txt`. Add `PNGS=dir PNG_AT=frames` to save the frames that differ.

## B1. Short checkbox and radio labels lose their last glyph (zgx core)

"Lamps" draws as "Lamp", "HUD" as "HU", "ACES" as "ACE"; "Sun shadows" and "Neutral" draw whole. A `text` beside a growing sibling does it too ("100%" as "100").

It reproduces on main `d7d9fec52`: `examples/repro-b1` on branch `lane/aaa-demo-repro-b1` (`9a65078e9`) measures a checkbox label's ink against a plain `text`'s and exits 1. The harbor's HUD shows the bug as it is.

## Waiting on planned primitives (not gaps)

| In the scene | Lacking | Milestone |
| --- | --- | --- |
| Dawn to night in one scene | Automatic exposure: the harbor is in physical units (sun in lux, lamps in lumens, at EV 11), and its lamps only read with the exposure moved by hand | R1 |
| Haze and depth: the far ridge reads as a dark wall | Height and distance fog, aerial perspective | R6 |
| A sky that follows the sun | The physical sky; the environment's prefilter on the GPU (a new sun costs 30–70 ms of CPU to relight, so the panel's sun slider moves the light, not the sky) | R6, R8 |
| Lamp heads and lit windows that glow | Bloom: their emission clips to white today | R1 |
| Rain landing on the water, the containers and the cranes | Collision with the depth buffer (`Collide::Depth`, design §5.4); today one plane, so drops over the water stop at the quay's height, 3 m above it | R7 |
| Rain hidden behind what's in front of it | Particles drawn against the scene's depth: today they draw over every mesh (`docs/3d.md`, Particles) | R7, the pass list |
| Rain that takes the lamps' and the sky's light | Lit particles: today they're unlit, so the rain's colour is set by hand on the harbor's physical scale | R7 |
| Water | Custom materials, compute for waves | R4 |
| Light shafts between the cranes | Froxel fog, density volumes | R6 |
| Bounce light under the gantries | Probes, GTAO | R8 |

## Closed

- **Lamps by the hundred:** 221 point and spot lights in lumens through clusters (stage 1). There were 8 nearest at stage 0.
- **Shadow detail over the quay:** four cascades over 600 m (stage 1).
- **Rain:** 45,000 streaks a second in a 160 m box following the camera, splashing where they land through a sub-emitter (stage 1).
