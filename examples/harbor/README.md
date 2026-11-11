# Harbor at dawn

The reference demo of zgx-3d's primitives (`docs/design-3d-aaa.md`, section 10): a container harbor at dawn, built on the **public API** of the zgx crates alone. The finished demo adds rain, water, volumetric light, particles and positional sound.

Everything is made by code here, from a seed: the quay, about 5,000 containers, the gantry cranes, the ship, the far shore, every texture and the dawn sky's light. Nothing is downloaded or copied from anywhere.

**Light is in physical units:**
- the sun in lux;
- 221 lamps in lumens (sodium high masts, quay posts, crane floods, deck lights), shaded through clusters;
- the sky's radiance and emissive surfaces on the same scale;
- all of it brought to the display by an exposure of EV 11 (`scene::EV`).

The sun's shadow is cut into four cascades over 600 m.

What the API can't express yet is in [GAPS.md](GAPS.md). Those gaps are reported, not worked around.

## Running

The crate is a workspace of its own (it isn't a member of zgx's), with its own lockfile seeded from the root's. Run it from this folder:

```sh
export CARGO_PROFILE_DEV_DEBUG=0
cargo run --release --bin harbor          # a window: the fly-through, or orbit with it off
cargo run --release --bin harbor_bench    # offscreen, a fixed path, numbers and proof
cargo test                                # incl. tests/public_api.rs, the public-API guard
```

**Why not a member:**
- membership would add it to the workspace's lockfile;
- `--workspace` builds would unify every zgx-3d feature it turns on into the other crates' builds.

Its `[patch.crates-io]` and profiles mirror the root's, so it builds against the same wgpu-hal zgx ships.

## The window

- The HUD shows frame rate, objects, triangles, containers and lights.
- In the settings panel, every control writes a signal, and the world follows by an effect (`settings.rs`).
- Turned off, "Fly-through" hands the camera to `OrbitControls`. An orbiting world left alone draws nothing.

## `harbor_bench`

It draws the app as it runs (HUD over world) through the public offscreen renderer `zgx_gpu::GpuRenderer`. `FRAMES` frames are spread evenly over the 60 s fly-through. Each frame is waited for by reading it back (GAPS.md, G5).

It prints:
- frames rendered, and frames where the world changed;
- instructions and CPU time per frame (macOS, `proc_pid_rusage`);
- frame-time percentiles, and the readback's own cost;
- GPU busy time, when built with the instrument (below);
- proof it drew: a hash of the last frame, the share of it unlike the clear colours, and a hash over every frame. The run fails if under a quarter of the frame was drawn.

The scene and the path are deterministic. Without cascades and clustered lights the frames' hash repeats run to run; with either, a few frames differ by a few pixels (GAPS.md, L3).

**Settings:**

| Setting | Default | What it does |
| --- | --- | --- |
| `FRAMES` | 600 | Frames measured |
| `WARM` | 30 | Warm-up frames before them |
| `WIDTH`, `HEIGHT` | 1920, 1080 | Size in device pixels |
| `SCALE` | 2 | Device pixels a point |
| `BLOCKS` | `5x7` | The yard's blocks |
| `SKY=0` | | Hemisphere light instead of the dawn sky |
| `SHADOWS=0` | | The sun casts no shadows |
| `CASCADES=0` | | One shadow map instead of cascades |
| `LAMPS=0` | | Every lamp at zero lumens |
| `LAMP_REACH` | 1 | Scales every lamp's range (0: none built) |
| `SUN` | 7 | The sun's elevation in degrees, its light and the sky's |
| `EXPOSURE` | 0 | Stops over the harbor's own (the panel's slider) |
| `HASHES=file` | | Each frame's hash, a line each |
| `PNG=file` | | Writes the last frame |
| `PNGS=dir` | | Writes six frames along the path (`PNG_AT=i,j` those frames instead) |

**GPU busy time** comes from the perf lane's instrument (`comparisons/3d/wgpu-hal-gpu-time.patch`). `./gpu_time.sh` builds the bench against a patched copy of the vendored wgpu-hal under the target directory, then runs it. Tracked files are left as they were.

## Layout

| File | What |
| --- | --- |
| `src/scene.rs` | The world: quay, yard, gantries, ship, lamps, shore |
| `src/shapes.rs` | Geometry made in code: beams, corrugated containers, the lofted hull, the ridge |
| `src/textures.rs` | Container wear, concrete, asphalt, from noise |
| `src/sky.rs` | The dawn sky as an equirect `Environment`, and the sun |
| `src/path.rs` | The fly-through (Catmull-Rom through key views, on frame time) |
| `src/settings.rs`, `src/hud.rs` | The panel's signals and their bindings; the interface |
| `src/perf.rs` | Process counters, percentiles, the frame hash |
| `tests/public_api.rs` | Fails if the demo names a `#[doc(hidden)]` item of zgx, zgx-3d, zgx-desktop or zgx-gpu, reaches into their files, or turns on a feature marked internal |
