#!/usr/bin/env python3
"""Draw one scene with each engine and compare what it cost.

Every arm draws the same glTF scene under one shadowed sun and one
environment, offscreen, at the same resolution, along the same camera path
(written here, one pose a line), and prints one `RESULT {json}` line: GPU ms
a frame (timestamps), frame time with two frames in flight, the CPU time of
every thread and the instructions retired a frame (the process's own
`proc_pid_rusage`), its footprint, and the proof that it drew (frames, a
hash of the last frame, the fraction of pixels unlike the clear colour, any
errors). Arms run interleaved, `--rounds` times, each round in a new order.

    python3 comparisons/3d/fetch.py            # the scenes, never committed
    python3 comparisons/3d/run.py --arms zgx,bevy,filament --rounds 5

Builds go to $ZGX_3D_TARGET (default ~/.cache/zgx-3d-comparison/target-*);
see README.md for each engine's settings and what is unequal between them.
"""
import argparse
import json
import math
import os
import random
import shutil
import statistics
import subprocess
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
CACHE = Path(os.environ.get("ZGX_3D_CACHE", Path.home() / ".cache" / "zgx-3d-comparison"))

# Scenes with licences that allow this use (see fetch.py). Bistro is
# Amazon Lumberyard's (CC-BY 4.0), converted from its FBX by fetch.py.
SCENES = {
    "bistro": {
        "model": "bistro/gltf/exterior/exterior.gltf",
        "sun": (-0.45, -1.0, 0.3),
    },
    "helmet": {
        "model": "khronos/Models/DamagedHelmet/glTF/DamagedHelmet.gltf",
        "sun": (-0.5, -1.0, -0.6),
    },
    # Generated in each arm (no file): N cubes of one mesh and material on a
    # 1.5 m grid, the first M bobbing every frame.
    "boxes-100k-still": {"model": "boxes:100000:0", "sun": (-0.45, -1.0, 0.3)},
    "boxes-10k-moving": {"model": "boxes:10000:10000", "sun": (-0.45, -1.0, 0.3)},
    # zgx only, for the instance and culling work: shapes it wasn't tuned on.
    # 5,000 meshes, each with its own material (every fourth textured).
    "materials-5k": {"model": "materials:5000", "sun": (-0.45, -1.0, 0.3)},
    # A tree 5 deep, 6 a node (7,776 leaf cubes under 1,555 groups); its 6
    # top subtrees turn every frame, moving every leaf through its parents.
    "hierarchy-7776": {"model": "hierarchy:5:6", "sun": (-0.45, -1.0, 0.3)},
}
ENVIRONMENT = "polyhaven/kloofendal_48d_partly_cloudy_puresky_2k.hdr"


def camera_path(scene, frames):
    """One pose a frame: position, then target."""
    poses = []
    for i in range(frames):
        t = i / max(frames - 1, 1)
        if scene == "bistro":
            # Half round the whole quarter above its roofs (the tallest reach 43.5 m),
            # looking at its middle.
            a = math.pi * (0.15 + t)
            poses.append(((25.0 + 70.0 * math.cos(a), 45.0 + 3.0 * math.sin(2 * a), 4.0 + 70.0 * math.sin(a)),
                          (25.0, 0.0, 4.0)))
        elif scene.startswith(("boxes", "materials", "hierarchy")):
            # Round the middle, low enough to see along the rows.
            radius = 100.0 if "100k" in scene else 70.0
            a = 2 * math.pi * t
            poses.append(((radius * math.cos(a), 30.0, radius * math.sin(a)), (0.0, 0.0, 0.0)))
        else:
            # Once round, 3 m out.
            a = 2 * math.pi * t
            poses.append(((3.0 * math.sin(a), 0.4, 3.0 * math.cos(a)), (0.0, 0.0, 0.0)))
    return poses


ARMS = {
    "zgx": {"dir": "zgx", "bin": "zgx-compare-3d"},
    # The same, its shadow map redrawn every frame as the others' are.
    # The light scene: 256 point lights over the model (Bistro), where zgx
    # (8 lights) can't take part yet.
    "bevy-lights": {"dir": "bevy", "bin": "bevy-compare-3d", "target": "bevy",
                    "args": ["--point-lights=256"]},
    "filament-lights": {"dir": "filament", "bin": "filament-compare-3d", "target": "filament",
                        "args": ["--point-lights=256"]},
    # Bevy's glTF loader makes no mipmaps for PNGs; this gives every image
    # a chain made as zgx makes its own, so texture cost is equal.
    "bevy-mips": {"dir": "bevy", "bin": "bevy-compare-3d", "target": "bevy", "args": ["--mips=1"]},
    # A/B arms that attribute the GPU time: no shadow pass; Bevy without its
    # GPU-driven path (preprocessing, culling, indirect draws).
    # The instrument's cost: the zgx arm on zgx's own wgpu-hal, unpatched
    # (no GPU spans). Its frame time and instructions must match zgx's.
    "zgx-plain": {"dir": ".zgx-plain", "bin": "zgx-compare-3d", "target": "zgx-plain"},
    # The zgx arm on another zgx tree ($ZGX_3D_CULL_TREE: a checkout of a
    # branch with frustum culling), culling on and off.
    "zgx-cull": {"dir": ".zgx-cull", "bin": "zgx-compare-3d", "target": "zgx-cull"},
    "zgx-cull-off": {"dir": ".zgx-cull", "bin": "zgx-compare-3d", "target": "zgx-cull",
                     "args": ["--culling=0"]},
    # Load time split by phase (adds a timed re-decode, kept out of load ms).
    "zgx-load-phases": {"dir": "zgx", "bin": "zgx-compare-3d", "target": "zgx", "args": ["--load-phases=1"]},
    # 256 point lights over Bistro as bevy-lights' and filament-lights',
    # clustered (zgx-3d's `lighting` feature).
    "zgx-lights": {"dir": "zgx", "bin": "zgx-compare-3d", "target": "zgx-lights",
                   "features": ["lighting"], "args": ["--point-lights=256"]},
    # Attribution for zgx-lights: the clustered shader with no lights, and
    # the lights reaching 1 m instead of 8.
    "zgx-clustered": {"dir": "zgx", "bin": "zgx-compare-3d", "target": "zgx-lights",
                      "args": ["--clustered=1"]},
    "zgx-lights-near": {"dir": "zgx", "bin": "zgx-compare-3d", "target": "zgx-lights",
                        "args": ["--point-lights=256", "--light-range=1"]},
    "zgx-noshadow": {"dir": "zgx", "bin": "zgx-compare-3d", "target": "zgx", "args": ["--shadows=0"]},
    "bevy-noshadow": {"dir": "bevy", "bin": "bevy-compare-3d", "target": "bevy", "args": ["--shadows=0"]},
    # Bevy's environment refiltered every frame (its realtime mode), as the
    # first runs had it; the default filters once.
    "bevy-env-realtime": {"dir": "bevy", "bin": "bevy-compare-3d", "target": "bevy",
                          "args": ["--env-realtime=1"]},
    "bevy-noindirect": {"dir": "bevy", "bin": "bevy-compare-3d", "target": "bevy",
                        "args": ["--no-indirect=1"]},
    "zgx-shadows": {"dir": "zgx", "bin": "zgx-compare-3d", "target": "zgx",
                    "args": ["--shadow-every-frame=1"]},
    # The post column: bloom, ACES and a grade through a 32³ table
    # (`report::warm_lut`) in each engine (zgx-3d's `post` feature, Bevy's
    # Bloom and ColorGrading, Filament's bloom and ColorGrading); and zgx
    # with the chain compiled in but off, against `zgx` (today's path).
    "zgx-post": {"dir": "zgx", "bin": "zgx-compare-3d", "target": "zgx-post",
                 "features": ["post"], "args": ["--post=bloom,lut"]},
    "zgx-post-off": {"dir": "zgx", "bin": "zgx-compare-3d", "target": "zgx-post"},
    "bevy-post": {"dir": "bevy", "bin": "bevy-compare-3d", "target": "bevy", "args": ["--post=bloom,lut"]},
    # With mipmaps (as bevy-mips): without them Bevy's textures alias into
    # bright HDR texels, which bloom then spreads over the frame.
    "bevy-mips-post": {"dir": "bevy", "bin": "bevy-compare-3d", "target": "bevy",
                       "args": ["--mips=1", "--post=bloom,lut"]},
    "filament-post": {"dir": "filament", "bin": "filament-compare-3d", "target": "filament",
                      "args": ["--post=bloom,lut"]},
    "bevy": {"dir": "bevy", "bin": "bevy-compare-3d"},
    "filament": {"dir": "filament", "bin": "filament-compare-3d"},
}


def trees():
    """`ZGX_3D_TREES=label=path,...`: zgx checkouts to run as arms `zgx@label`."""
    found = {}
    for item in filter(None, os.environ.get("ZGX_3D_TREES", "").split(",")):
        label, path = item.split("=", 1)
        found[label] = Path(path).resolve()
    return found


for _label in trees():
    ARMS[f"zgx@{_label}"] = {"dir": f".zgx-tree-{_label}", "bin": "zgx-compare-3d",
                             "target": f"zgx@{_label}"}


def tree_zgx(arm):
    """The zgx arm's sources against the checkout `zgx@label` names, its
    culling hooks on where that tree has them."""
    label = arm.split("@", 1)[1]
    tree = trees()[label]
    copy = HERE / f".zgx-tree-{label}"
    copy.mkdir(exist_ok=True)
    manifest = (HERE / "zgx" / "Cargo.toml").read_text()
    manifest = manifest.replace('path = "../../../crates/', f'path = "{tree}/crates/')
    culls = "pub fn set_culling" in (tree / "crates/zgx-3d/src/world.rs").read_text()
    if culls:
        manifest = manifest.replace('default = ["gpu-time"]', 'default = ["gpu-time", "cull-tree"]')
    (copy / "Cargo.toml").write_text(manifest)
    if not (copy / "src").exists():
        (copy / "src").symlink_to(HERE / "zgx" / "src")
    lock = HERE / "zgx" / "Cargo.lock"
    if lock.exists() and not (copy / "Cargo.lock").exists():
        shutil.copy(lock, copy / "Cargo.lock")


def target_dir(arm):
    arm = ARMS[arm].get("target", arm)
    return Path(os.environ.get("ZGX_3D_TARGET", CACHE)) / f"target-{arm}"


def timed_wgpu_hal():
    """zgx's vendored wgpu-hal with wgpu-hal-gpu-time.patch, for the wgpu arms."""
    copy = HERE / ".wgpu-hal-timed"
    stamp = copy / ".patched"
    patch = HERE / "wgpu-hal-gpu-time.patch"
    if stamp.exists() and stamp.read_text() == patch.read_text():
        return
    if copy.exists():
        shutil.rmtree(copy)
    shutil.copytree(HERE.parent.parent / "vendor" / "wgpu-hal", copy)
    subprocess.run(["patch", "-s", "-p1", "-d", str(copy), "-i", str(patch)], check=True)
    stamp.write_text(patch.read_text())


def plain_zgx():
    """The zgx arm's sources, built against the unpatched wgpu-hal."""
    copy = HERE / ".zgx-plain"
    copy.mkdir(exist_ok=True)
    manifest = (HERE / "zgx" / "Cargo.toml").read_text()
    manifest = manifest.replace('path = "../.wgpu-hal-timed"', 'path = "../../../vendor/wgpu-hal"')
    manifest = manifest.replace('default = ["gpu-time"]', 'default = []')
    (copy / "Cargo.toml").write_text(manifest)
    if (copy / "src").is_symlink() or (copy / "src").exists():
        (copy / "src").unlink() if (copy / "src").is_symlink() else shutil.rmtree(copy / "src")
    (copy / "src").symlink_to(HERE / "zgx" / "src")
    lock = HERE / "zgx" / "Cargo.lock"
    if lock.exists() and not (copy / "Cargo.lock").exists():
        shutil.copy(lock, copy / "Cargo.lock")


def cull_zgx():
    """The zgx arm's sources, built against the zgx tree at $ZGX_3D_CULL_TREE."""
    tree = Path(os.environ["ZGX_3D_CULL_TREE"]).resolve()
    copy = HERE / ".zgx-cull"
    copy.mkdir(exist_ok=True)
    manifest = (HERE / "zgx" / "Cargo.toml").read_text()
    manifest = manifest.replace('path = "../../../crates/', f'path = "{tree}/crates/')
    manifest = manifest.replace('default = ["gpu-time"]', 'default = ["gpu-time", "cull-tree"]')
    (copy / "Cargo.toml").write_text(manifest)
    if not (copy / "src").exists():
        (copy / "src").symlink_to(HERE / "zgx" / "src")
    lock = HERE / "zgx" / "Cargo.lock"
    if lock.exists() and not (copy / "Cargo.lock").exists():
        shutil.copy(lock, copy / "Cargo.lock")


def build(arm):
    target = target_dir(arm)
    if arm == "filament":
        sdk = CACHE / "filament" / "filament"
        subprocess.run(
            ["cmake", "-S", HERE / "filament", "-B", target, "-G", "Ninja",
             "-DCMAKE_BUILD_TYPE=Release", f"-DFILAMENT_DIR={sdk}"],
            check=True,
        )
        subprocess.run(["cmake", "--build", target], check=True)
        return target / ARMS[arm]["bin"]
    timed_wgpu_hal()
    if arm == "zgx-plain":
        plain_zgx()
    if arm == "zgx-cull":
        cull_zgx()
    if arm.startswith("zgx@"):
        tree_zgx(arm)
    # The tree arms share one target (their dependencies are the same) and
    # each copies its binary out to its own.
    shared = arm.startswith("zgx@")
    build_target = target.parent / "target-zgx-trees" if shared else target
    env = dict(os.environ, CARGO_TARGET_DIR=str(build_target), CARGO_PROFILE_DEV_DEBUG="0")
    # The plain copy's lock differs from the arm's by wgpu-hal's source alone.
    locked = [] if arm in ("zgx-plain", "zgx-cull") or arm.startswith("zgx@") else ["--locked"]
    features = ["--features", ",".join(ARMS[arm]["features"])] if "features" in ARMS[arm] else []
    subprocess.run(["cargo", "build", "--release", "--bin", ARMS[arm]["bin"], *locked, *features],
                   cwd=HERE / ARMS[arm]["dir"], env=env, check=True)
    if shared:
        (target / "release").mkdir(parents=True, exist_ok=True)
        shutil.copy2(build_target / "release" / ARMS[arm]["bin"], target / "release" / ARMS[arm]["bin"])
    return target / "release" / ARMS[arm]["bin"]


def run_arm(binary, arm, scene, args, out_dir, round_):
    path_file = out_dir / f"{scene}-path.txt"
    png = out_dir / f"{scene}-{arm}-{round_}.png"
    command = [
        str(binary),
        "--model=" + (SCENES[scene]["model"] if SCENES[scene]["model"].startswith(("boxes:", "materials:", "hierarchy:"))
                      else str(CACHE / SCENES[scene]["model"])),
        f"--environment={CACHE / ENVIRONMENT}",
        f"--path={path_file}",
        f"--width={args.width}",
        f"--height={args.height}",
        f"--warmup={args.warmup}",
        f"--msaa={args.msaa}",
        "--sun=" + ",".join(str(v) for v in SCENES[scene]["sun"]),
        f"--out={png}",
    ]
    if ARMS[arm].get("target", arm) == "filament":
        command.append(f"--ibl={CACHE / 'filament-ibl'}")
    command += ARMS[arm].get("args", [])
    if args.quick and ARMS[arm].get("target", arm).startswith("zgx"):
        command += ["--rest-s=0", "--rest-ticks=0"]
    done = subprocess.run(command, capture_output=True, text=True, timeout=900)
    log = out_dir / f"{scene}-{arm}-{round_}.log"
    log.write_text(done.stdout + "\n--- stderr ---\n" + done.stderr)
    results = [line[len("RESULT "):] for line in done.stdout.splitlines() if line.startswith("RESULT ")]
    if done.returncode != 0 or not results:
        raise SystemExit(f"{arm} failed (exit {done.returncode}); see {log}")
    result = json.loads(results[-1])
    result["png"] = str(png)
    return result


def check(result, frames):
    """The arm drew: every frame counted, a picture in the last, no errors."""
    problems = []
    if result.get("frames") != frames:
        problems.append(f"counted {result.get('frames')} frames of {frames}")
    if (result.get("drawn_fraction") or 0) < 0.25:
        problems.append(f"only {result.get('drawn_fraction')} of the last frame drawn")
    if result.get("errors"):
        problems.append("errors: " + result["errors"])
    return problems


KEYS = [
    ("gpu_ms_per_frame", "GPU busy ms/frame", 3),
    ("gpu_busy_fraction", "GPU busy share", 2),
    ("gpu_ms_median", "GPU ms median", 3),
    ("gpu_ms_p95", "GPU p95", 3),
    ("frame_ms_median", "frame ms", 3),
    ("frame_ms_p95", "frame p95", 3),
    ("fps", "fps", 1),
    ("cpu_ms_per_frame", "CPU ms/frame", 3),
    ("submit_ms_median", "submit ms", 3),
    ("instructions_per_frame", "Minstr/frame", 2),
    ("footprint_mb", "footprint MB", 1),
    ("peak_footprint_mb", "peak MB", 1),
    ("load_ms", "load ms", 0),
    ("load_peak_footprint_mb", "load peak MB", 1),
    ("rested_footprint_mb", "rest 10 s MB", 1),
    ("rest_frames", "frames at rest", 0),
    ("gpu_marker_ms_median", "GPU (markers)", 3),
    ("gpu_spans", "GPU spans", 0),
    ("drawn_meshes_median", "zgx meshes drawn", 0),
    ("visible_meshes_median", "meshes in frustum", 0),
    ("visible_triangles_median", "tris in frustum", 0),
]


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--arms", default="zgx,bevy,filament")
    parser.add_argument("--scene", default="bistro", choices=SCENES)
    parser.add_argument("--rounds", type=int, default=5)
    parser.add_argument("--frames", type=int, default=300)
    parser.add_argument("--warmup", type=int, default=60)
    parser.add_argument("--msaa", type=int, default=4, choices=(1, 4))
    parser.add_argument("--width", type=int, default=1920)
    parser.add_argument("--height", type=int, default=1080)
    parser.add_argument("--out", default=str(CACHE / "runs" / "latest"))
    parser.add_argument("--no-build", action="store_true")
    parser.add_argument("--quick", action="store_true",
                        help="zgx arms: skip the rest measurements (footprint and frames at rest)")
    args = parser.parse_args()
    arms = args.arms.split(",")
    out_dir = Path(args.out)
    out_dir.mkdir(parents=True, exist_ok=True)
    with open(out_dir / f"{args.scene}-path.txt", "w") as f:
        f.write("# position xyz, target xyz\n")
        for (p, t) in camera_path(args.scene, args.frames):
            f.write(" ".join(f"{v:.5f}" for v in (*p, *t)) + "\n")
    binaries = {}
    built = {}
    for arm in arms:
        binaries[arm] = target_dir(arm) / ("" if ARMS[arm].get("target", arm) == "filament" else "release") / ARMS[arm]["bin"]
        key = ARMS[arm].get("target", arm)
        if not args.no_build:
            built[key] = built.get(key) or build(key)
            binaries[arm] = built[key]
    results = {arm: [] for arm in arms}
    order = list(arms)
    for round_ in range(args.rounds):
        random.Random(round_).shuffle(order)
        for arm in order:
            result = run_arm(binaries[arm], arm, args.scene, args, out_dir, round_)
            problems = check(result, args.frames)
            if problems:
                raise SystemExit(f"{arm} round {round_} didn't prove it drew: {problems}")
            results[arm].append(result)
            print(f"round {round_} {arm}: gpu {result.get('gpu_ms_median')} ms, "
                  f"frame {result.get('frame_ms_median')} ms", file=sys.stderr)
    (out_dir / f"{args.scene}-results.json").write_text(json.dumps(results, indent=1))
    print(f"{args.scene} {args.width}x{args.height}, {args.frames} frames, "
          f"{args.rounds} rounds: median of the rounds' medians (min-max)")
    print(f"{'':16}" + "".join(f"{arm:>24}" for arm in arms))
    for key, label, digits in KEYS:
        row = f"{label:16}"
        for arm in arms:
            values = [r[key] for r in results[arm] if r.get(key) is not None]
            if key == "instructions_per_frame":
                values = [v / 1e6 for v in values]
            if not values:
                row += f"{'-':>24}"
                continue
            cell = f"{statistics.median(values):.{digits}f} ({min(values):.{digits}f}-{max(values):.{digits}f})"
            row += f"{cell:>24}"
        print(row)
    for arm in arms:
        r = results[arm][-1]
        print(f"{arm}: {r.get('adapter')}; last frame hash {r.get('hash')}, "
              f"{r.get('drawn_fraction'):.3f} drawn, {r['png']}")


if __name__ == "__main__":
    main()
