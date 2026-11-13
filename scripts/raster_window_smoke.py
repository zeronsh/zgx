#!/usr/bin/env python3
"""A window drawn by the software raster (`ZGX_RENDERER=raster`) shows what
the GPU renderer shows: the scroll example on an owned X11 desktop, once on
the GPU (lavapipe in CI) and once on the raster, each captured as it opens
and again after the same wheel scrolls. Each pair matches but for edge
antialiasing; the raster's run presents through it (`raster present` in its
present log) and renders nothing on the GPU, and the GPU's the reverse. A
third run (`ZGX_RENDERER=auto`) takes the raster where the machine has no
render node (`/dev/dri/renderD*`, CI's), and the GPU where it has one.

The menus example's menu, opened by a click, shows on the raster as on the
GPU: the GPU's in a popup window of its own (X11), the raster's in the
window (popup windows are drawn by GPU renderers), the screen the same.
In the window it can't pass the window's edge: with the window made too
short for it below its trigger or above, the raster's menu shifts to lie
whole inside the window (`compose_popover::placement`), not cut off.

    cargo build --release -p zgx-desktop --example scroll
    python3 scripts/raster_window_smoke.py target/release/examples/scroll \
        --menus target/release/examples/menus --output /tmp/zgx-raster-window
"""
import argparse
import os
import pathlib
import re
import select
import subprocess
import time

from PIL import Image, ImageChops

from platform_smoke import stop, wait_for_wm

TITLE = "zgx retained scroll"
# How far a pixel may differ (of 255, in its worst channel), and on what
# share of the window: against lavapipe its worst channel differed by 4,
# blend rounding (rounded corners' edges, text); shapes' edges elsewhere
# differ by up to ~30 on a tenth of a percent. A frame drawn wrong (text in
# another font, a scroll left unmoved) differs on several percent.
TOLERANCE = 16
SHARE = 0.001


def desktop(processes, output):
    """An owned Xvfb and Openbox: the client's environment."""
    log = (output / "xvfb.log").open("w")
    xvfb = subprocess.Popen(["Xvfb", "-r", "-noreset", "-displayfd", "1", "-screen", "0", "1280x900x24", "-ac"],
                            stdout=subprocess.PIPE, stderr=log, text=True)
    processes.append(xvfb)
    if not select.select([xvfb.stdout], [], [], 30)[0]:
        raise RuntimeError("Xvfb startup timed out")
    env = dict(os.environ, DISPLAY=":" + xvfb.stdout.readline().strip(), WINIT_X11_SCALE_FACTOR="1",
               WINIT_UNIX_BACKEND="x11", XDG_RUNTIME_DIR="/tmp",
               DBUS_SESSION_BUS_ADDRESS="unix:path=/nonexistent")
    env.pop("WAYLAND_DISPLAY", None)
    processes.append(subprocess.Popen(["openbox"], env=env, stdout=log, stderr=log))
    wait_for_wm(env)
    return env


def capture(env, window, path):
    subprocess.run(["import", "-window", window, str(path)], env=env, check=True)
    return Image.open(path).convert("RGB")


def run_menu(binary, env, output, renderer):
    """The menus example's first menu opened: the screen before and after."""
    log_path = output / f"menu-{renderer}.log"
    with log_path.open("w") as log:
        app = subprocess.Popen([str(binary)], env=dict(env, ZGX_RENDERER=renderer, ZGX_PRESENT_LOG="1"),
                               stdout=log, stderr=log)
        try:
            window = None
            for _ in range(200):
                found = subprocess.run(["xdotool", "search", "--onlyvisible", "--pid", str(app.pid)],
                                       env=env, text=True, capture_output=True)
                if found.returncode == 0 and found.stdout.strip():
                    window = found.stdout.split()[0]
                    break
                if app.poll() is not None:
                    raise RuntimeError(f"menu {renderer}: the example exited; see {log_path}")
                time.sleep(.05)
            else:
                raise RuntimeError(f"menu {renderer}: no window")
            subprocess.run(["xdotool", "windowactivate", "--sync", window], env=env, check=True)
            time.sleep(1)
            closed = capture(env, "root", output / f"menu-{renderer}-closed.png")
            subprocess.run(["xdotool", "mousemove", "--window", window, "80", "84", "click", "1"], env=env, check=True)
            # Away from the menu, so no item shows hovered.
            subprocess.run(["xdotool", "mousemove", "1000", "740"], env=env, check=True)
            time.sleep(1)
            opened = capture(env, "root", output / f"menu-{renderer}-opened.png")
        finally:
            stop(app)
    if ImageChops.difference(closed, opened).getbbox() is None:
        raise RuntimeError(f"menu {renderer}: no menu showed")
    return opened


# The menus example's menu background, and its height (`menus.rs`).
MENU_COLOR = (0x36, 0x51, 0x6e)
MENU_HEIGHT = 156


def menu_rows(image):
    """The longest run of rows showing the menu's background."""
    width, height = image.size
    pixels = image.load()
    best = run = 0
    for y in range(height):
        if any(pixels[x, y] == MENU_COLOR for x in range(0, width, 2)):
            run += 1
            best = max(best, run)
        else:
            run = 0
    return best


def run_menu_at_edge(binary, env, output):
    """The menus example on the raster, its window made 200 points tall (too
    short for the menu below its trigger or above): the window with the
    menu open."""
    log_path = output / "menu-edge.log"
    with log_path.open("w") as log:
        app = subprocess.Popen([str(binary)], env=dict(env, ZGX_RENDERER="raster"), stdout=log, stderr=log)
        try:
            window = None
            for _ in range(200):
                found = subprocess.run(["xdotool", "search", "--onlyvisible", "--pid", str(app.pid)],
                                       env=env, text=True, capture_output=True)
                if found.returncode == 0 and found.stdout.strip():
                    window = found.stdout.split()[0]
                    break
                time.sleep(.05)
            else:
                raise RuntimeError("menu at the edge: no window")
            subprocess.run(["xdotool", "windowsize", "--sync", window, "500", "200"], env=env, check=True)
            subprocess.run(["xdotool", "windowactivate", "--sync", window], env=env, check=True)
            time.sleep(1)
            subprocess.run(["xdotool", "mousemove", "--window", window, "80", "84", "click", "1"], env=env, check=True)
            subprocess.run(["xdotool", "mousemove", "1000", "740"], env=env, check=True)
            time.sleep(1)
            opened = capture(env, window, output / "menu-edge.png")
        finally:
            stop(app)
    return opened


def run(binary, env, output, renderer):
    """The example on `renderer`: its window captured as it opens and after
    three wheel steps down, and its present log."""
    log_path = output / f"{renderer}.log"
    with log_path.open("w") as log:
        app = subprocess.Popen([str(binary)], env=dict(env, ZGX_RENDERER=renderer, ZGX_PRESENT_LOG="1"),
                               stdout=log, stderr=log)
        try:
            window = None
            for _ in range(200):
                found = subprocess.run(["xdotool", "search", "--onlyvisible", "--name", f"^{TITLE}$"],
                                       env=env, text=True, capture_output=True)
                if found.returncode == 0:
                    window = found.stdout.split()[-1]
                    break
                if app.poll() is not None:
                    raise RuntimeError(f"{renderer}: the example exited ({app.returncode}); see {log_path}")
                time.sleep(.05)
            else:
                raise RuntimeError(f"{renderer}: no window")
            # Its first frames drawn, and the pointer over its content.
            time.sleep(1.5)
            subprocess.run(["xdotool", "windowactivate", "--sync", window, "mousemove", "--window", window, "170", "200"],
                           env=env, check=True)
            time.sleep(.3)
            opened = capture(env, window, output / f"{renderer}-opened.png")
            # Real (XTEST) wheel clicks: winit ignores ones sent to a window.
            for _ in range(3):
                subprocess.run(["xdotool", "click", "5"], env=env, check=True)
                time.sleep(.15)
            # The scroll's easing settled.
            time.sleep(1.5)
            scrolled = capture(env, window, output / f"{renderer}-scrolled.png")
        finally:
            stop(app)
    return opened, scrolled, log_path.read_text(errors="replace").splitlines()


def compare(gpu, raster, what, output):
    if gpu.size != raster.size:
        raise RuntimeError(f"{what}: the windows differ in size, {gpu.size} and {raster.size}")
    difference = ImageChops.difference(gpu, raster)
    worst = [max(pixel) for pixel in difference.getdata()]
    off = sum(d > TOLERANCE for d in worst) / len(worst)
    print(f"{what}: worst channel {max(worst)}, {off * 100:.3f}% of pixels more than {TOLERANCE} off")
    if off > SHARE:
        difference.point(lambda v: 255 if v > TOLERANCE else 0).save(output / f"{what}-difference.png")
        raise RuntimeError(f"{what}: {off * 100:.3f}% of pixels differ by more than {TOLERANCE}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=pathlib.Path)
    parser.add_argument("--menus", type=pathlib.Path, required=True)
    parser.add_argument("--output", type=pathlib.Path, required=True)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    binary = args.binary.resolve()
    processes = []
    try:
        env = desktop(processes, args.output)
        gpu_opened, gpu_scrolled, gpu_log = run(binary, env, args.output, "gpu")
        raster_opened, raster_scrolled, raster_log = run(binary, env, args.output, "raster")
        rendered = re.compile(r"PRESENT \S+ render forced=")
        presented = re.compile(r"PRESENT \S+ raster present rects=")
        gpu_frames = sum(bool(rendered.search(line)) for line in gpu_log)
        raster_frames = sum(bool(presented.search(line)) for line in raster_log)
        print(f"GPU: {gpu_frames} frames rendered; raster: {raster_frames} frames presented")
        if gpu_frames == 0 or any(presented.search(line) for line in gpu_log):
            raise RuntimeError("the GPU run rendered no frame on the GPU")
        if raster_frames < 4 or any(rendered.search(line) for line in raster_log):
            raise RuntimeError(f"the raster run presented {raster_frames} raster frames, or rendered on the GPU")
        if ImageChops.difference(raster_opened, raster_scrolled).getbbox() is None:
            raise RuntimeError("the raster window's content didn't move under the wheel")
        compare(gpu_opened, raster_opened, "opened", args.output)
        compare(gpu_scrolled, raster_scrolled, "scrolled", args.output)
        _, _, auto_log = run(binary, env, args.output, "auto")
        software = not any(node.name.startswith("renderD") for node in pathlib.Path("/dev/dri").glob("*"))
        auto_raster = any(presented.search(line) for line in auto_log)
        auto_gpu = any(rendered.search(line) for line in auto_log)
        print(f"auto: {'raster' if auto_raster else 'GPU' if auto_gpu else 'nothing'} "
              f"({'no render node' if software else 'a render node'})")
        if (auto_raster, auto_gpu) != (software, not software):
            raise RuntimeError("auto drew on the wrong renderer for this machine")
        menu_gpu = run_menu(args.menus.resolve(), env, args.output, "gpu")
        menu_raster = run_menu(args.menus.resolve(), env, args.output, "raster")
        compare(menu_gpu, menu_raster, "menu", args.output)
        whole = menu_rows(menu_gpu)
        edge = run_menu_at_edge(args.menus.resolve(), env, args.output)
        shown = menu_rows(edge)
        print(f"menu at the window's edge: {shown} of its {whole} rows shown, the window {edge.size[1]} tall")
        if whole < MENU_HEIGHT - 8:
            raise RuntimeError(f"the GPU's menu showed {whole} rows of {MENU_HEIGHT}: not the menu sought")
        if shown < whole - 2:
            raise RuntimeError(f"the raster's menu at the window's edge was cut off: {shown} of {whole} rows")
        print("PASS: the raster window shows what the GPU's does: opened, scrolled, a menu open")
    finally:
        for process in reversed(processes):
            stop(process)


if __name__ == "__main__":
    main()
