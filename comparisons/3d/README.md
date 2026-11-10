# 3D comparison: zgx-3d, Bevy, Filament

The same scene drawn by each renderer and what each costs. Every arm uses:
- the same resolution and camera path;
- the same sun and environment.

This is the bar the 3D design sets for its primitives (`docs/design-3d-aaa.md`, section 2a). Nothing here is a dependency of any zgx crate: each arm is its own Cargo workspace or CMake project.

## Running a table

Every block of the published table is one command. A later milestone reruns the same blocks and adds its own arm as a column; a new primitive adds a scene of its own (below).

```sh
export ZGX_3D_CACHE=~/.cache/zgx-3d-comparison ZGX_3D_TARGET=~/.cache/zgx-3d-comparison
python3 comparisons/3d/fetch.py        # scenes, Filament, the converted Bistro (pinned below)
python3 comparisons/3d/run.py --rounds 3 --scene boxes-100k-still --arms zgx,zgx-plain,bevy,bevy-noindirect,filament
python3 comparisons/3d/run.py --rounds 3 --scene boxes-10k-moving --arms zgx,bevy,filament
python3 comparisons/3d/run.py --rounds 3 --scene bistro --arms zgx,zgx-plain,zgx-shadows,zgx-noshadow,bevy,bevy-mips,bevy-noshadow,bevy-noindirect,bevy-env-realtime,filament
python3 comparisons/3d/run.py --rounds 3 --scene bistro --msaa 1 --arms zgx,bevy,filament
python3 comparisons/3d/run.py --rounds 3 --scene bistro --arms zgx,zgx-lights,zgx-clustered,bevy,bevy-lights,filament,filament-lights
```

A zgx branch joins as the `zgx-cull` and `zgx-cull-off` arms with `ZGX_3D_CULL_TREE=<its checkout>`; the arm reads `World::set_culling` and `World::culled` there.

**Run on a quiet machine:**
- Rounds are interleaved, in a new order each round, but other work moves every number.
- Record the load and the busiest processes for each block. The published run's script did this every 30 s and flagged any arm whose GPU busy time spread more than 5% across rounds.

### Pinned

| What | Version | Check |
| --- | --- | --- |
| Bevy | 0.19.1 (the release on wgpu 29, as zgx) | `bevy/Cargo.lock` |
| Filament | v1.80.0 macOS release | sha256 in `fetch.py` |
| assimp (Bistro's conversion, run only) | 6.0.5 (Homebrew; it reports "6.0") | `fetch.py` checks `assimp version` |
| Bistro | v5.2, `Bistro_v5_2.zip` | sha256 in `fetch.py` |
| Khronos glTF-Sample-Assets (DamagedHelmet) | commit `8f492390` | `fetch.py` |
| Environment | Poly Haven Kloofendal 48d Partly Cloudy (Pure Sky), 2K HDR, CC0 | md5 in `fetch.py` |
| zgx's wgpu-hal, as the wgpu arms build it | `vendor/wgpu-hal` + `wgpu-hal-gpu-time.patch` | `run.py` |

**Needs:**
- `fetch.py`: git; for Bistro, assimp on `PATH` and Pillow.
- The Filament arm: CMake and Ninja (on macOS it builds against the release's prebuilt libraries).
- Bevy's build is about 1.5 GB.

## Scenes

Nothing fetched is committed. Each scene's licence allows rendering it here.

| Scene | What | Licence |
| --- | --- | --- |
| `bistro` | Amazon Lumberyard Bistro, exterior: 2.83 M triangles, 1,591 mesh primitives (1,296 glTF meshes), 132 materials, 405 textures. A half orbit over the roofs, looking at the quarter's middle. | CC-BY 4.0: "Amazon Lumberyard Bistro, Open Research Content Archive (ORCA)", Amazon Lumberyard, 2017, [developer.nvidia.com/orca/amazon-lumberyard-bistro](https://developer.nvidia.com/orca/amazon-lumberyard-bistro) |
| `helmet` | Khronos DamagedHelmet: one mesh, every map. Once round it, 3 m out. | CC-BY 4.0 |
| `boxes-100k-still` | Generated in each arm: 100,000 cubes of one mesh and material on a 1.5 m grid, the camera circling. | Ours |
| `boxes-10k-moving` | 10,000 cubes, every one moving every frame. | Ours |
| Bistro + `--point-lights=256` | 256 point lights over Bistro. The lighting milestone's scene. | as Bistro |

Still to come, one per milestone:
- a post-chain scene: bloom, tone map and grading, which Bevy and Filament both have;
- a TAA scene.

Bistro ships as FBX with DDS textures. `convert_bistro.py` converts it once with assimp (run, not linked; its version goes in the glTF's `asset.generator`). Its material and texture handling:

- **Base colour:** the alpha cuts holes as `MASK` at 0.5.
- **Specular map:** red is occlusion, green roughness, blue metal. It is used as glTF's metallicRoughness map. Its occlusion is dropped: the placeholder maps hold 0 there, which would take away all ambient light.
- **Normal maps:** DirectX convention, so green is flipped for glTF.
- **Texture size:** every texture is written as a PNG at most 1024². No arm compresses PNG textures on the GPU, and at 2048² each would hold several GB.

Every arm reads the same converted files.

Sponza is not here: the Khronos copy's files are under the CRYENGINE Limited License Agreement, which grants use for games made with CRYENGINE alone.

## What each arm does

| | zgx (`zgx/`) | Bevy (`bevy/`) | Filament (`filament/`) |
| --- | --- | --- | --- |
| GPU API | wgpu 29 / Metal, zgx's vendored wgpu-hal | wgpu 29 / Metal, the same wgpu-hal | Metal |
| Loop | a world's painter drawn into an offscreen target on its own device, two frames in flight | Bevy's default app: pipelined rendering, multi-threaded, no-wait schedule runner, an `Image` render target | `beginFrame`/`render`/`endFrame` on a headless readable swap chain; a refused `beginFrame` waits 50 µs and asks again, as a vsync-paced app would wait |
| Antialiasing | 4x MSAA | 4x MSAA | 4x MSAA (its FXAA and dithering off) |
| Tone map | ACES (Hill's fit) | ACES fitted (Hill's fit) | ACES |
| Exposure | 1 | ev100 = log2(1/1.2), so 1 | aperture 1, shutter 1.2 s, ISO 100, so 1 |
| Sun | 3 lux, one 2048² shadow map, 4 taps | 3 lux, 1 cascade of 2048², Hardware2x2 | 3 lux, 1 cascade of 2048², PCF |
| Environment | prefiltered on the CPU at load: 128² cube and SH9 | `GeneratedEnvironmentMapLight` (256² cube) filtered on the GPU once, then kept | cmgen's 256² KTX and SH9, prefiltered offline |
| HDR target | none: tone map and sRGB encode in the forward pass | none (non-HDR camera) | yes: an HDR target, then a tone-map pass |

## Equal work, scene by scene

"Equal work" means:
- the same triangles, textures, lights and shadow map;
- the same environment, filtered once;
- the same resolution and multisampling;
- the same camera path.

Each renderer is free to skip what it can prove invisible (culling) or unchanged (zgx's kept frames). Where an arm does different work, it's listed here; an attribution arm puts it right where one exists.

**Every scene:**
- **Environment filtering:** Bevy's `GeneratedEnvironmentMapLight` refilters every frame (its realtime mode: about 1–2 ms of GPU a frame on the M5 Max). The Bevy arm filters once, then keeps the filtered cubes; `bevy-env-realtime` keeps refiltering.
- **Shadow map:** zgx redraws its shadow map only when the sun or a caster moves; Bevy and Filament redraw theirs every frame. `zgx-shadows` redraws it every frame.
- **HDR target:** Filament draws to an HDR target with a tone-map pass. zgx and Bevy tone map in the forward pass.
- **Environment cube size:** zgx 128², Bevy and Filament 256².
- **Shadow filtering:** zgx 4 taps, Bevy Hardware2x2, Filament PCF.
- **Frames at rest:** zgx draws none when nothing changes. Bevy's loop draws every tick; Filament draws whenever the app calls it (here every tick).

**Bistro:**
- **Mipmaps:** Bevy's glTF loader makes no mipmaps for PNGs, so it samples level 0 everywhere; zgx and Filament build chains. `bevy-mips` gives Bevy the same chains.
- **Meshes and culling:** both zgx and Bevy draw a primitive as a mesh: 1,591 of them. On this path Bevy's frustum keeps a median 1,471, and so does zgx-cull's. Bevy culls by frustum on the CPU and again on the GPU. zgx on main culls nothing and draws all 1,591; `zgx-cull` culls by frustum on the CPU.
- **Point lights:** the 256 point lights are 150 lm each, reaching 8 m, in every arm: zgx's `zgx-lights` shades them through clusters (zgx-3d's `lighting` feature; its arm `zgx` without the lights is the base). zgx's are given in candela, 150 / 4π.

**Boxes:**
- **Batching:** one mesh and one material for every cube in every arm. Bevy and zgx batch them into instanced draws. Filament draws a renderable per cube (gltfio's nodes): its natural API without instancing, so its CPU numbers carry that.
- **Moving boxes:** each arm moves the 10,000 boxes through its own API every frame. zgx: `set_transform`. Bevy: `Transform`. Filament: `TransformManager`.

## What's measured

All counts are read from inside each process. No tool attaches to it, but `footprint -v` may read it from outside.

| Column | How |
| --- | --- |
| GPU busy ms/frame | The time the GPU was busy, per measured frame: every command buffer's `[GPUStartTime, GPUEndTime]` over the measured frames, their union divided by the frames. Metal reports these times; overlap between frames counts once. Filament reports its own `gpuFrameDuration`, from the same command-buffer times, per frame. **The wgpu arms (zgx, Bevy) read them from a patched copy of zgx's vendored wgpu-hal:** `wgpu-hal-gpu-time.patch`, in this folder. `run.py` copies `vendor/wgpu-hal` to `.wgpu-hal-timed/` (ignored by git) and applies the patch, and only these arms build against it; nothing shipped changes. `zgx-plain` is the zgx arm on the unpatched wgpu-hal, to show the instrument costs nothing: on Bistro it retires 23.58 M instructions a frame against zgx's 23.59 M. |
| GPU busy share | busy time over the span from the first command buffer's start to the last one's end: under 1 when the GPU waited on the CPU |
| GPU ms median, GPU (markers) | secondary instruments, kept beside the headline (see below) |
| frame ms | wall time between frames, at most two in flight |
| CPU ms/frame | the process's CPU time on every thread (`proc_pid_rusage`), per measured frame |
| Minstr/frame | instructions retired by the process, per frame (`proc_pid_rusage`) |
| footprint MB | physical footprint, as `footprint` counts it, while drawing |
| load ms, load peak MB | from the process's start to the first frame drawn with everything loaded (Bevy: and every pipeline compiled); the footprint's peak by then |
| at rest 10 s MB | the footprint after ten seconds at rest. zgx: idle, as it is when nothing changes. Bevy and Filament: their loops ticking at 120 Hz. |
| frames at rest | frames drawn over those ticks with nothing changing. zgx's are counted in a zgx window's own frame loop (a headless renderer). |

**Proof each arm drew:**
- every measured frame is counted;
- the last frame is written as a PNG and hashed;
- the fraction of its pixels unlike the clear colour is reported;
- errors are reported.

`run.py` refuses a round with a wrong frame count, under a quarter of the frame drawn, or an error.

### Why GPU time is busy time

Two instruments were tried first and dropped:

- **Timestamp markers.** These are empty passes holding a timestamp, before and after the frame's passes. A pass that shares no resource with the frame's passes can run beside them, so a bracket can start late or end early. On Bistro, Bevy's per-pass markers (shadows 0.90 ms, main pass 1.49 ms) summed past its bracketed total (0.68 ms).
- **Per-submission spans.** This is each submission's first command buffer start to its last end. When the GPU runs one frame's work beside the next, spans overlap. `bevy-noindirect` gave 7.0 ms of span per frame at 4.1 ms of wall time per frame, which can't be busy time.

The union of command-buffer intervals counts each moment the GPU worked once, whatever overlapped.

## Attribution arms

| Arm | Differs by |
| --- | --- |
| `zgx-shadows` | the sun nudged every frame, so the shadow map is redrawn every frame (as the others') |
| `zgx-noshadow`, `bevy-noshadow` | no sun shadows |
| `bevy-noindirect` | Bevy's CPU path: `NoIndirectDrawing` (no GPU preprocessing, culling or indirect draws) |
| `bevy-mips` | every image given a mip chain on the CPU after load (the 2x2 mean zgx uses) |
| `bevy-env-realtime` | Bevy's environment refiltered every frame |
| `zgx-plain` | zgx's own wgpu-hal, unpatched: the GPU-time instrument's cost |
| `zgx-cull`, `zgx-cull-off` | another zgx tree (`ZGX_3D_CULL_TREE`), culling on and off |
| `bevy-lights`, `filament-lights`, `zgx-lights` | 256 point lights over the scene (zgx: clustered, the `lighting` feature) |
| `zgx-clustered` | the clustered shader with no point lights: what clusters cost by themselves |
| `zgx-lights-near` | `zgx-lights` with each light reaching 1 m: what the lights a pixel shades cost |

`--msaa=1` runs any of them without multisampling.

## Results

### Nov 10 2026, M5 Max (macOS 26.4)

**Conditions:**
- **Machine:** quiet of builds, with system processes at about 4 cores throughout (PerfPowerServices ~197%, Finder ~100%, mds_stores ~99%; 1-minute load 6–8.7). Frame ms and CPU ms may carry that. Instructions (exact to ±0.5%) and GPU busy time are the robust rows.
- **Disturbed blocks:**
  - Another lane's tests and build ran 20:10–20:19 UTC. The 10k-boxes block was rerun after the others.
  - Bistro's two arms whose first round fell in that time and stood out (`bevy-env-realtime` +13.6%, Filament +7.1%) were rerun. Their reruns are below.
  - Every zgx arm in Bistro stayed within 0.4% round to round.
- **Trees:**
  - zgx main: `eb9694955`.
  - zgx-cull: `2102434e`, the graph lane's branch (kept instances, kept frames on camera-only changes, dirty-range uploads, frustum culling by spans). It is a different commit, **not merged**.
- **Builds:** Bevy 0.19.1, Filament v1.80.0, Bistro converted by assimp 6.0.5.
- **Method:** 1920x1080 offscreen, 300 frames after 60 of warm-up, 3 rounds interleaved; medians of the rounds' medians.

**Where zgx loses:**
- **Load time against Filament on Bistro:** 2.05 s against 1.24 s. Filament's real figure is lower (about 0.8 s): its arm read files a byte at a time until after this run.
  - zgx's time goes mostly to decoding its 405 PNGs on one thread (about 1.6–1.8 s).
  - Its first frame's CPU mip chains and uploads take about 0.5 s.
  - Its environment prefilter takes only ~40 ms.
  - Filament decodes in parallel and builds mips on the GPU (`loadResources` 0.31 s).
  - Fix tracked in #1954; split by `zgx-load-phases`.
- **Box scenes on main:** the 100k scene's first frame takes main 3.15 s, the largest number in the tables, against the branch's 79 ms. #1967 fixes it (add's slot scan).
- **Elsewhere:** zgx leads on GPU, CPU, memory at rest and frames at rest.

**100,000 still boxes, camera circling.** Boxes drawn a frame (median): zgx-cull 30,274, against the 36,260 Bevy's frustum kept; zgx main and culling off draw all 100,000.

| | zgx main | zgx-cull | zgx-cull, culling off | Bevy | Bevy, CPU path | Filament |
| --- | --- | --- | --- | --- | --- | --- |
| GPU busy ms/frame | 2.69 | 0.325 | 0.497 | 1.39 | 1.06 | 6.02 |
| Minstr/frame | 111.6 | 4.77 | 1.22 | 119.3 | 116.8 | 347.8 |
| frame ms | 6.96 | 0.31 | 0.47 | 2.89 | 2.43 | 10.67 |
| footprint MB | 621 | 635 | 624 | 1130 | 1102 | 1186 |
| at rest after 10 s, MB | 380 | 409 | 398 | 1123 | 1047 | 1096 |
| load ms | 3150 | 79 | 74 | 368 | 353 | 238 |

**10,000 boxes, every one moving.** zgx-cull draws a median 8,263 of them. GPU busy varied 15–31% round to round in every arm, in both runs: at a third of a millisecond a frame, the GPU's clocks move it. Read the instructions.

| | zgx main | zgx-cull | Bevy | Filament |
| --- | --- | --- | --- | --- |
| GPU busy ms/frame | 0.36 | 0.35 | 0.78 | 1.14 |
| Minstr/frame | 14.65 | 15.06 | 47.6 | 63.7 |
| frame ms | 0.69 | 0.77 | 2.15 | 1.70 |
| footprint MB | 495 | 495 | 904 | 537 |
| at rest after 10 s, MB | 268 | 268 | 905 | 534 |

**Bistro exterior, MSAA 4.** Meshes drawn a frame (median): zgx-cull and Bevy's frustum 1,471 of 1,591 (Bevy: 2.43 M triangles). zgx main draws all 1,591.

| | GPU busy ms/frame | Minstr/frame | frame ms | footprint MB | at rest 10 s MB | load ms | load peak MB |
| --- | --- | --- | --- | --- | --- | --- | --- |
| zgx main | 1.17 | 23.6 | 1.39 | 2699 | 2274 | 2048 | 2689 |
| zgx-cull | 1.08 | 17.4 | 1.13 | 2693 | 2275 | 1987 | 2688 |
| zgx, shadow map every frame | 1.64 | 34.1 | 1.96 | 2890 | 2198 | 1996 | 2689 |
| zgx, no shadows | 1.14 | 22.9 | 1.36 | 2685 | 2258 | 1990 | 2611 |
| Bevy | 3.00 | 60.1 | 3.87 | 7102 | 4227 | 2075 | 3637 |
| Bevy with mip chains | 2.59 | 59.3 | 3.55 | 6989 | 4611 | 2648 | 5047 |
| Bevy, no shadows | 2.19 | 39.3 | 2.40 | 4522 | 3625 | 2059 | 3609 |
| Bevy, CPU path | 2.38 | 42.2 | 2.56 | 4249 | 3512 | 2095 | 3625 |
| Bevy, environment refiltered every frame (rerun) | 4.30 | 73.1 | 4.64 | 8225 | 4612 | 2117 | 3626 |
| Filament (rerun) | 2.09 | 38.1 | 2.47 | 3025 | 2993 | 1242 | 4926 |

**Bistro, compared at equal work** (shadow map every frame, mip chains everywhere):
- **Overall:** zgx 1.64 ms, Filament 2.09, Bevy 2.59.
- **Forward pass without shadows:** zgx 1.14 against Bevy's 2.19.
- **A shadow redraw:** zgx 0.46 ms, Bevy 0.81.
- **Bevy's GPU-driven path:** costs it 0.63 ms over its CPU path on this GPU.

**Bistro at MSAA 1:** zgx 1.19, zgx-cull 1.11, Bevy 2.88, Filament 1.95 ms of GPU a frame. On this tile-based GPU, 4x MSAA costs zgx nothing measurable.

**256 point lights over Bistro** (the lighting milestone's targets):
- Bevy (clustered forward): 3.17 ms GPU, 61.5 M instructions a frame (+0.17 ms, +1.3 M over its base).
- Filament (froxels): 2.35 ms, 39.5 M (+0.26 ms, +1.3 M).
- zgx shades 8 lights and isn't in this table.

### Withdrawn (Nov 10, rough, loaded machine)

The first Sponza and Bistro tables timed zgx's and Bevy's GPU with timestamp markers only, which can run beside the passes they bracket. Their Bevy runs also refiltered the environment every frame. **Their zgx and Bevy GPU columns are withdrawn.** Their CPU, instruction, memory, load and idle rows stand. Sponza isn't published here: its licence doesn't allow it.
