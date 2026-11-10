# What the harbor needed that the public API can't express

Each entry is a primitive any game would need, in general form, with the smallest API that would close it. None is worked around in the demo: where one bites, the scene shows the flaw, and this file says so. Entries are filed with the 3D work's coordinator as they're found.

Status: **open** (nothing covers it yet), **planned** (the design has it; the demo waits for its milestone), **closed** (landed; the demo uses it).

## Gaps

### G1. A world's frame statistics are hidden — open (planned in design §7 as `World::stats()`)

**Needed by:** any app with a performance HUD, any benchmark, any test of "a still world draws nothing".

`World::frames_drawn`, `World::culled` and `World::set_culling` exist, but are `#[doc(hidden)]` ("for tests and benches"). So does `World::viewport()`, the painter `world_bench` draws through. An app can't read its frames drawn, draws, culled meshes or instances.

`harbor_bench` therefore counts "frames with the world changed" by `World::revision()`. That's inferred, not observed: it can't tell a frame the renderer skipped from one it drew.

```rust
pub struct Stats { pub frames: u64, pub draws: u32, pub instances: u32, pub culled: u32, pub triangles: u64 }
impl World { pub fn stats(&self) -> Stats; }   // the last frame's, plus frames so far
```

### G2. The environment can't be drawn as the background — open

**Needed by:** any game with an HDR sky or skybox (an equirect, a cube, a captured probe).

`set_background` takes one flat colour; the environment lights and reflects but is never seen. Design §6.5 makes only the *physical* sky a background, so an app's own sky image (a painted dawn, a nebula, an interior cube) has no way to be seen.

The harbor's background is a flat lilac, `sky::BACKGROUND`. The horizon behind the cranes doesn't match the light on them.

```rust
pub enum Background { None, Color([f32; 4]), Environment { blur: f32 } }
impl World { pub fn set_background_of(&self, background: Background); }
```

### G3. Per-instance data beyond colour and emission — planned (design §5.1, the `instance` four floats, R4)

**Needed by:** any instanced crowd: wear, dirt, damage, ID tints, animation offsets.

Today an instance varies by its material's colour and emission only. Containers vary in hue; their wear can't vary per box (one wear texture for all of them).

### G4. A mesh can't opt out of the sun's shadow, so large ones spread it thin — open

**Needed by:** any outdoor scene with a ground plane, water, far terrain or a backdrop.

The sun's one shadow map is fitted to every opaque mesh (`render/shadows.rs`, `sun_bounds`). A 6 km water plane and a far ridge, neither of which needs to cast, stretch its 2048² over kilometres.

Measured on the harbor (`SHADOWS=0` against `SHADOWS=1`, pixels changed by more than 8/255 in the frame's middle, 1920×1080):

| t | the harbor as built | water 700 m, no ridge |
| --- | --- | --- |
| 40 s | 4.6% | 17.6% |
| 50 s | 7.0% | 10.6% |
| 10 s | 0.0% | 0.2% |

So the stacks' shadows on each other mostly vanish.

- **Cascades (R3)** fit the view instead, which fixes the resolution.
- **The opt-out is still its own primitive:** a ground, water or sky mesh casting into every cascade costs draw time, and a backdrop's shadow is wrong in kind.

```rust
impl World {
    pub fn set_casts_shadows(&self, id: ObjectId, on: bool);    // default true
    pub fn set_receives_shadows(&self, id: ObjectId, on: bool); // default true
}
```

### G5. The offscreen renderer has no wait but a readback — open (zgx-gpu)

**Needed by:** offscreen benches, video renderers, anything pacing frames without a window.

`GpuRenderer` submits and returns. The only public way to wait for a frame is to read it back (`readback`, `readback_into`). The harbor's frame times therefore include a readback, measured apart (about 0.7–1.3 ms at 1080p on the M1 Max) and printed beside them.

```rust
impl GpuRenderer { pub fn finish(&self) -> Result<(), GpuError>; } // wait for submitted frames
```

## Waiting on planned primitives (not gaps)

What the scene lacks today, and the milestone it waits for. These are known to the design, not new needs.

| In the scene | Lacking | Milestone |
| --- | --- | --- |
| Haze and depth: the far ridge reads as a dark wall | Height and distance fog, aerial perspective | R6 |
| A sky that follows the sun | The physical sky; the environment's prefilter on the GPU (a new sun angle costs 30–70 ms of CPU to relight today, so the sun slider moves the light, not the sky) | R6, R8 |
| All 14 lamps lit | 8 point lights shaded a frame (the nearest the camera's target) | R3 |
| Dawn grading, bloom on lamps and windows | The post chain | R1 |
| Water | Custom materials (displace, scene colour and depth), compute for waves | R4 |
| Rain, splashes | Particles | R7 |
| Light shafts between the cranes | Froxel fog, density volumes | R6 |
| Bounce light under the gantries | Probes, GTAO | R8 |
| Moving cranes | Animator (the trolley and spreader are groups ready for it) | R10 |

## Engine bugs found (not primitives)

**B1. Short labels lose their last glyph (zgx core).** Seen in `checkbox` and `radio` labels, and in a `text` beside a growing sibling in a `row`:
- "Lamps" draws as "Lamp", "HUD" as "HU", "ACES" as "ACE", "100%" as "100";
- "Sun shadows", "Neutral" and a lone `text("Lamps")` in a column draw whole.

It happens:
- at scale 1 and 2;
- with an inherited `text_size` and without one;
- in an absolutely placed box too.

Drawn by the offscreen `GpuRenderer`; a window is unverified. A standalone repro was made outside the repository; it's described in the report that filed this. The harbor's HUD shows the bug as it is.
