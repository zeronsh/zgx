#!/usr/bin/env python3
"""A window repaints only what changed: the frame_clock example, its box
moving each frame across a 520x240 window, as a native Wayland client under
an owned Sway (`--wayland`) or on an owned X11 desktop:

- its present log (`ZGX_PRESENT_LOG`) renders nearly every frame after the
  first few unforced (`render forced=false`): on Wayland a frame the
  compositor grants, and damage that waited for it, used to repaint the whole
  window every frame;
- its GPU stats (`ZGX_GPU_STATS`) damage a small part of the window a frame;
- `ZGX_DAMAGE_CHECK`, rendering each frame whole too and comparing, finds no
  stale pixel.

    cargo build --release -p zgx-desktop --example frame_clock
    python3 scripts/partial_repaint_smoke.py target/release/examples/frame_clock --wayland --output /tmp/zgx-partial
"""
import argparse
import os
import pathlib
import re
import select
import subprocess
import tempfile
import time

from platform_smoke import stop, wait_for_wm
from sway_host import wait_for_sway_host, sway_config

TITLE = "zgx frame clock"
# The window's area, in megapixels, at scale 1.
WINDOW_MP = 520 * 240 / 1e6
SECONDS = 4


def desktop(private, processes, output, wayland):
    """An owned Xvfb and Openbox, and with `wayland` a Sway on it: the
    client's environment."""
    env = dict(os.environ, XDG_RUNTIME_DIR=private, XDG_CONFIG_HOME=private + "/config",
               DBUS_SESSION_BUS_ADDRESS="unix:path=/nonexistent")
    for key in ("DISPLAY", "WAYLAND_DISPLAY", "SWAYSOCK", "I3SOCK"):
        env.pop(key, None)

    def launch(name, command, environment, **options):
        log = (output / (name + ".log")).open("w")
        child = subprocess.Popen(command, env=environment, stdout=options.pop("stdout", log),
                                 stderr=options.pop("stderr", log), **options)
        processes.append(child)
        return child

    xvfb = launch("xvfb", ["Xvfb", "-r", "-noreset", "-displayfd", "1", "-screen", "0", "1280x900x24", "-ac"],
                  env, stdout=subprocess.PIPE, text=True)
    if not select.select([xvfb.stdout], [], [], 30)[0]:
        raise RuntimeError("Xvfb startup timed out")
    env["DISPLAY"] = ":" + xvfb.stdout.readline().strip()
    launch("openbox", ["openbox"], env)
    wait_for_wm(env)
    if not wayland:
        return dict(env, WINIT_UNIX_BACKEND="x11")
    env.update(WLR_BACKENDS="x11", WLR_X11_OUTPUTS="1", WLR_RENDERER="pixman")
    config = pathlib.Path(private, "sway.conf")
    sway_config(config, f"xwayland disable\nseat seat0 fallback true\noutput * mode 1200x800\n"
                        f"default_border none\nfor_window [title=\"{TITLE}\"] floating enable\n")
    sway = launch("sway", ["sway", "--unsupported-gpu", "--config", str(config)], env)
    wait_for_sway_host(env, output, sway)
    deadline = time.monotonic() + 10
    while True:
        sockets = list(pathlib.Path(private).glob("sway-ipc*.sock"))
        displays = [p for p in pathlib.Path(private).glob("wayland-*") if p.suffix != ".lock"]
        if sockets and displays:
            break
        if time.monotonic() > deadline:
            raise RuntimeError("Sway startup timed out")
        time.sleep(.05)
    client = dict(env, SWAYSOCK=str(sockets[0]), WAYLAND_DISPLAY=displays[0].name, WINIT_UNIX_BACKEND="wayland")
    client.pop("DISPLAY")
    return client


def run(binary, env, output, name, extra):
    """The example for `SECONDS`, its stderr's lines."""
    log = output / f"{name}.log"
    with log.open("w") as stderr:
        app = subprocess.Popen([str(binary)], env=dict(env, **extra), stdout=stderr, stderr=stderr)
        try:
            app.wait(timeout=SECONDS)
            raise RuntimeError(f"{name}: the example exited early ({app.returncode}); see {log}")
        except subprocess.TimeoutExpired:
            pass
        finally:
            stop(app)
    return log.read_text(errors="replace").splitlines()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=pathlib.Path)
    parser.add_argument("--output", type=pathlib.Path, required=True)
    parser.add_argument("--wayland", action="store_true")
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    binary = args.binary.resolve()
    processes = []
    with tempfile.TemporaryDirectory(prefix="zgx-partial-repaint-") as private:
        try:
            env = desktop(private, processes, args.output, args.wayland)
            lines = run(binary, env, args.output, "frames", {"ZGX_PRESENT_LOG": "1", "ZGX_GPU_STATS": "1"})
            forced = [m.group(1) == "true" for m in map(re.compile(r"render forced=(\w+)").search, lines) if m]
            # The first frames are whole (the first, a resize as the window maps).
            later = forced[5:]
            if len(later) < 30:
                raise RuntimeError(f"only {len(forced)} frames rendered in {SECONDS} s")
            whole = sum(later)
            mpx = [(int(m.group(1)), float(m.group(2))) for m in
                   map(re.compile(r"GPU_STATS frames=(\d+) .*damaged_mpx=([\d.]+)").search, lines) if m]
            # Each second's sum over its frames, the first second's left out.
            per_frame = [mp / frames for frames, mp in mpx[1:] if frames]
            print(f"{len(forced)} frames rendered, {whole} of the {len(later)} after the first five whole; "
                  f"damaged per frame: {', '.join(f'{mp:.3f}' for mp in per_frame)} MP "
                  f"(the window is {WINDOW_MP:.3f} MP)")
            if whole > len(later) // 10:
                raise RuntimeError(f"{whole} of {len(later)} frames repainted the whole window")
            if per_frame and max(per_frame) > WINDOW_MP / 2:
                raise RuntimeError(f"frames damaged up to {max(per_frame):.3f} MP of {WINDOW_MP:.3f}")
            checked = run(binary, env, args.output, "check", {"ZGX_DAMAGE_CHECK": "1",
                                                              "ZGX_DAMAGE_DUMP": str(args.output)})
            stale = [line for line in checked if "ZGX_DAMAGE_CHECK frame" in line]
            if stale:
                raise RuntimeError(f"{len(stale)} frames left stale pixels: {stale[:3]}")
            print(f"PASS: partial frames{' on Wayland' if args.wayland else ' on X11'}, no stale pixel")
        finally:
            for process in reversed(processes):
                stop(process)


if __name__ == "__main__":
    main()
