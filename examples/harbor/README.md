# Harbor at dawn

The reference demo of zgx-3d's primitives (`docs/design-3d-aaa.md`, section 10): a container harbor at dawn, built on the **public API** of the zgx crates alone. The finished demo adds rain, water, volumetric light, particles and positional sound.

Everything is made by code here, from a seed: the quay, about 5,000 containers, the gantry cranes, the ship, the far shore, every texture and the dawn sky's light. Nothing is downloaded or copied from anywhere.

**Light is in physical units:**
- the sun in lux;
- 221 lamps in lumens (sodium high masts, quay posts, crane floods, deck lights), shaded through clusters;
- the sky's radiance and emissive surfaces on the same scale;
- all of it brought to the display by an exposure of EV 11 (`scene::EV`).

The sun's shadow is cut into four cascades over 600 m. It rains (`weather.rs`): particle streaks in a box that follows the camera, splashing where they land. The four gantry cranes work: each is a model the demo writes in code as a `.glb`, placed with an animator playing its cycle. The trolley runs out over the ship and back, and the spreader lowers and hoists.

**The post chain** blooms the lamps and windows and lets exposure adapt, so one scene holds from a hazy dawn to a lamp-lit night (the panel's exposure is then compensation).

**The water and the wet** are the demo's own, on the public API (`water.rs`, `shaders/`):
- the sea's waves are a compute pass: 64 wind waves on a 128 m tile's lattice, summed each frame into a field (a `GpuTexture`) by the sea's own clock, so a paused sea stops its passes and goes on where it stood;
- the water is a custom material reading that field: band-limited by each pixel's footprint (what it can't hold goes into roughness, so nothing aliases and the sun's glint is a soft path), roughness from the wind, murky, refraction a small bend that fades out at grazing and distance;
- a surface layer making everything wet while it rains, with puddles on the ground.

**Sound** (`sound.rs`, `sounds.rs`) is placed on objects through zgx-3d's spatial audio:
- each crane's drive on its trolley;
- the ship's generator at its funnel;
- a bell on a buoy in the channel;
- rain as a bed;
- reverb zones under the gantries and in the container canyons.

Every sound is synthesised in code. The window plays it through the system's output when a person runs it. `HARBOR_MUTE=1` (and `--smoke-test`) mixes it offline instead, opening no device. The bench always mixes offline, at master gain 1, and reports the master mix's RMS and peak.

**Containers wear their own way:** a custom material reads four numbers each container has (`World::set_instance_floats`: rust, grime, fading, a seed).

**Bounced light:** irradiance probes fill the apron under the gantries and the yard's canyons (`World::add_probes`). They're traced by ray queries, which the bench and the window ask for (`zgx_gpu::request_ray_queries`, unsafe: wgpu's experimental feature).

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

It installs a `zgx::clock::VirtualClock` and moves it a fixed step a frame, so whatever moves by itself is placed by the path's time.

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
| `RAIN=0` | | No rain made |
| `SOUND=0` | | No sound (otherwise mixed offline, no device) |
| `PROBES=0` | | No probes |
| `WEAR=0` | | Every container worn alike (the stock material) |
| `SEA=still` | | The sea's waves stopped (its passes off) |
| `PATH=still` | | The camera held at the path's start |
| `HUD=0` | | The world's view alone, for stills |
| `WATER` | | `stock`: the plain material, no wet; `dry`: no wet layer; `opaque`: no refraction |
| `WET=0` | | Rain, but nothing made wet |
| `POST=0` | | No post chain (exposure fixed at EV 11, no bloom) |
| `AUTO=0` | | Bloom, but exposure fixed |
| `CRANES` | `working` | `idle`: animators playing nothing; `still`: no animators |
| `GANTRIES` | 4 | Cranes on the quay |
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
| `src/crane.rs`, `src/glb.rs` | The gantry crane, written in code as a `.glb` with its work as a clip, placed with an animator each |
| `src/weather.rs` | The rain, on particle emitters |
| `src/water.rs`, `src/shaders/` | The sea and the wet, as custom materials |
| `src/sound.rs`, `src/sounds.rs` | Spatial sound on the harbor's objects; the sounds, synthesised |
| `src/textures.rs` | Container wear, concrete, asphalt, from noise |
| `src/sky.rs` | The dawn sky as an equirect `Environment`, and the sun |
| `src/path.rs` | The fly-through (Catmull-Rom through key views, on frame time) |
| `src/settings.rs`, `src/hud.rs` | The panel's signals and their bindings; the interface |
| `src/perf.rs` | Process counters, percentiles, the frame hash |
| `tests/public_api.rs` | Fails if the demo names a `#[doc(hidden)]` item of zgx, zgx-3d, zgx-desktop or zgx-gpu, reaches into their files, or turns on a feature marked internal |
