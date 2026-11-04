#!/usr/bin/env python3
"""A popup's glass on Linux blurs what its window drew under it
(`GpuRenderer::set_backdrop`): popup_probe's translucent popover over hard
stripes (`POPUP_PROBE_GLASS`), on X11 (Xvfb/Openbox/picom) and Wayland (headless
sway), read from the screen.

Over the window, a row of the popover is even, and brighter than its fill
alone: the stripes, blurred. Without the blur X11 shows the fill alone (nothing
composites its window with what's under it) and Wayland shows the stripes
sharp through it. Reaching past the window's bottom edge, a row over the window
and a row past it are both even and as bright: the window's edge goes on."""
import argparse
import json
import os
import pathlib
import select
import subprocess
import sys
import tempfile
import time

from PIL import Image

sys.path.insert(0, str(pathlib.Path(__file__).parent))
from platform_smoke import stop, wait_for_wm  # noqa: E402
from sway_host import sway_config  # noqa: E402

# popup_probe's window at 1x (`WindowBounds::new`'s pixels), its anchor 16 in,
# 16 up and 28 tall, the popover 200 by 120 at 4 from it.
WINDOW = (840, 440)
ANCHOR_TOP = WINDOW[1] - 16 - 28
# The fill alone over black: 0x11 at 65%.
FILL = 11
LEFT, RIGHT = 40, 190


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("probe", type=pathlib.Path, help="the popup_probe example")
    parser.add_argument("--output", type=pathlib.Path, required=True)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    processes, checks = [], {}

    def launch(name, command, env, **options):
        log = (args.output / (name + ".log")).open("w")
        child = subprocess.Popen(command, env=env, stdout=options.pop("stdout", log), stderr=log, **options)
        processes.append(child)
        return child

    def row(image, origin, y):
        """The green levels of the popover's row `y`, in window points."""
        return [image.getpixel((origin[0] + x, origin[1] + y))[1] for x in range(LEFT, RIGHT)]

    def judge(name, image, origin, y, where):
        levels = row(image, origin, y)
        low, high = min(levels), max(levels)
        checks[f"{name}: {where} row {y}"] = [low, high]
        return high - low <= 12 and low >= FILL + 20

    def shots(name, env, shot, origin_of, focus=lambda: None):
        for glass in ("over", "past"):
            app = launch(f"{name}-{glass}", [str(args.probe.resolve())],
                         dict(env, POPUP_PROBE_GLASS=glass))
            time.sleep(1)
            focus()
            # It opens by itself 4 s in, the window focused by then.
            time.sleep(6)
            assert app.poll() is None, f"{name} {glass}: the probe exited"
            image = shot(f"{name}-{glass}")
            origin = origin_of(image)
            if glass == "over":
                # 120 tall, ending 4 above the anchor.
                ok = judge(name, image, origin, ANCHOR_TOP - 4 - 60, "over the window")
                checks[f"{name}: glass over the window blurs what's under it"] = ok
            else:
                # 4 below the anchor, with 12 points over the window: the
                # middle of them, whose blur reads the window alone.
                ok = judge(name, image, origin, WINDOW[1] - 6, "at the edge")
                checks[f"{name}: glass reaching past the edge blurs the part over the window"] = ok
                past = judge(name, image, origin, WINDOW[1] + 60, "past the edge")
                checks[f"{name}: past the edge the glass goes on"] = past
                # Outside its bottom left corner, rounded: what's behind it, not
                # the window's pixels (a frame on a surface mixes the blur with
                # a copy of its own, not of what it blurs).
                corner = image.getpixel((origin[0] + 16, origin[1] + ANCHOR_TOP + 28 + 4 + 119))[1]
                behind = image.getpixel((origin[0] + 10, origin[1] + ANCHOR_TOP + 28 + 4 + 119))[1]
                checks[f"{name}: corner and what's behind it"] = [corner, behind]
                checks[f"{name}: outside its corner it shows what's behind it"] = abs(corner - behind) <= 12
            stop(app)
            processes.remove(app)

    try:
        with tempfile.TemporaryDirectory() as stage:
            # X11.
            env = dict(os.environ, WINIT_X11_SCALE_FACTOR="1", XDG_RUNTIME_DIR=stage,
                       DBUS_SESSION_BUS_ADDRESS="unix:path=/nonexistent")
            env.pop("WAYLAND_DISPLAY", None)
            xvfb = launch("xvfb", ["Xvfb", "-r", "-noreset", "-displayfd", "1", "-screen", "0", "1280x1024x24", "-ac"],
                          env, stdout=subprocess.PIPE, text=True)
            if not select.select([xvfb.stdout], [], [], 30)[0]:
                raise RuntimeError("Xvfb startup timed out")
            env["DISPLAY"] = ":" + xvfb.stdout.readline().strip()
            openbox = launch("openbox", ["openbox"], env)
            wait_for_wm(env)
            # A compositor: with none, the system reports reduced transparency,
            # and glass paints opaque, unblurred.
            config = args.output / "picom.conf"
            config.write_text('backend = "xrender"; shadow = false; fading = false; unredir-if-possible = false;\n')
            picom = launch("picom", ["picom", "--config", str(config)], env)
            time.sleep(.5)
            assert picom.poll() is None, "picom exited"

            def x_shot(name):
                path = args.output / f"{name}.png"
                subprocess.run(["import", "-window", "root", str(path)], env=env, check=True)
                return Image.open(path).convert("RGB")

            def x_origin(_):
                found = subprocess.run(["xdotool", "search", "--onlyvisible", "--name", "Popup probe"],
                                       env=env, capture_output=True, text=True).stdout.split()
                info = subprocess.run(["xdotool", "getwindowgeometry", "--shell", found[0]],
                                      env=env, capture_output=True, text=True).stdout
                values = dict(line.split("=") for line in info.split())
                return int(values["X"]), int(values["Y"])

            def x_focus():
                found = subprocess.run(["xdotool", "search", "--onlyvisible", "--name", "Popup probe"],
                                       env=env, capture_output=True, text=True).stdout.split()
                subprocess.run(["xdotool", "windowactivate", "--sync", found[0]], env=env, check=True)

            shots("x11", env, x_shot, x_origin, x_focus)
            stop(picom)
            stop(openbox)
            stop(xvfb)
            processes.clear()

            # Wayland: the window at the output's corner, undecorated.
            runtime = pathlib.Path(stage, "sway")
            runtime.mkdir(mode=0o700)
            config = runtime / "sway.conf"
            sway_config(config, "output * mode 1280x960\ndefault_border none\n"
                                "for_window [app_id=\".*\"] floating enable, move position 0 0\n")
            env = dict(os.environ, XDG_RUNTIME_DIR=str(runtime), WLR_BACKENDS="headless", WLR_RENDERER="pixman",
                       WLR_LIBINPUT_NO_DEVICES="1", DBUS_SESSION_BUS_ADDRESS="unix:path=/nonexistent")
            env.pop("DISPLAY", None)
            launch("sway", ["sway", "--unsupported-gpu", "-c", str(config)], env)
            deadline = time.monotonic() + 10
            while not (sockets := [p for p in runtime.glob("wayland-*") if p.suffix != ".lock"]):
                assert time.monotonic() < deadline, "no sway socket"
                time.sleep(.05)
            env["WAYLAND_DISPLAY"] = sockets[0].name

            def wayland_shot(name):
                path = args.output / f"{name}.png"
                subprocess.run(["grim", str(path)], env=env, check=True)
                return Image.open(path).convert("RGB")

            shots("wayland", env, wayland_shot, lambda _: (0, 0))
    finally:
        for process in reversed(processes):
            stop(process)
    (args.output / "checks.json").write_text(json.dumps(checks, indent=1))
    print(json.dumps(checks, indent=1))
    failed = [name for name, ok in checks.items() if ok is False]
    if failed:
        sys.exit("FAIL: " + "; ".join(failed))


main()
