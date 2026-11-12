#!/usr/bin/env python3
"""A window covering its display on Wayland (`WindowOptions::covers_display`,
the agents_intro example) under an owned nested Sway whose output is scaled
1, 2 and 1.5 (fractional): the window is the output's size in the
compositor's logical pixels, as xdg-output says it, from the first size it
asks for (its fixed minimum and maximum, before its first commit), and shows
over the whole output.

Before, it asked for the output's mode in logical pixels (winit takes a
physical size at the scale a window has as it is first configured, 1 where
the compositor has said none by then, as Hyprland hasn't): at 1.5, half as
large again as the output."""
from sway_host import wait_for_openbox, sway_config
import argparse
import json
import os
import pathlib
import re
import select
import subprocess
import tempfile
import time

from platform_smoke import stop

MODE = (1200, 800)
SCALES = (1, 2, 1.5)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=pathlib.Path)
    parser.add_argument("--output", type=pathlib.Path, required=True)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    (args.output / "results.json").unlink(missing_ok=True)
    compositor = app = xvfb = wm = None
    with tempfile.TemporaryDirectory(prefix="zgx-covers-display-") as runtime:
        os.chmod(runtime, 0o700)
        config = pathlib.Path(runtime, "sway.conf")
        sway_config(config, "xwayland disable\nseat seat0 fallback true\n"
                            "output * mode %dx%d\ndefault_border none\n" % MODE)
        (args.output / "sway.conf").write_text(config.read_text())
        env = dict(os.environ, XDG_RUNTIME_DIR=runtime, WLR_BACKENDS="x11", WLR_X11_OUTPUTS="1",
                   WLR_RENDERER="pixman")
        for name in ("DISPLAY", "WAYLAND_DISPLAY", "SWAYSOCK", "I3SOCK"):
            env.pop(name, None)
        try:
            with (args.output / "xvfb.log").open("w") as log:
                xvfb = subprocess.Popen(["Xvfb", "-r", "-noreset", "-displayfd", "1", "-screen", "0",
                                         "1400x1000x24", "-ac"], stdout=subprocess.PIPE, stderr=log, text=True)
                if not select.select([xvfb.stdout], [], [], 30)[0]:
                    raise RuntimeError("owned Xvfb startup timed out")
                env["DISPLAY"] = ":" + xvfb.stdout.readline().strip()
            with (args.output / "openbox.log").open("w") as log:
                wm = subprocess.Popen(["openbox"], env=env, stdout=log, stderr=log)
            wait_for_openbox(env, args.output, wm)
            with (args.output / "sway.log").open("w") as log:
                compositor = subprocess.Popen(["sway", "--unsupported-gpu", "--config", str(config)],
                                              env=env, stdout=log, stderr=log)
            deadline = time.monotonic() + 8
            while True:
                sockets = list(pathlib.Path(runtime).glob("sway-ipc*.sock"))
                displays = [p for p in pathlib.Path(runtime).glob("wayland-*") if p.suffix != ".lock"]
                if sockets and displays:
                    break
                if time.monotonic() > deadline or compositor.poll() is not None:
                    raise RuntimeError("owned compositor startup failed")
                time.sleep(.05)
            env.update(WAYLAND_DISPLAY=displays[0].name, SWAYSOCK=str(sockets[0]))

            def ipc(*parts):
                return json.loads(subprocess.check_output(["swaymsg", "-s", str(sockets[0]), *map(str, parts)],
                                                          env=env, text=True))

            def window_of(pid):
                found = []

                def walk(node):
                    if node.get("pid") == pid:
                        found.append(node)
                    for child in node.get("nodes", []) + node.get("floating_nodes", []):
                        walk(child)
                walk(ipc("-t", "get_tree"))
                return found[0] if found else None

            output_name = ipc("-t", "get_outputs")[0]["name"]
            client_env = dict(env, WAYLAND_DEBUG="1", WINIT_UNIX_BACKEND="wayland")
            client_env.pop("DISPLAY", None)
            stages = []
            for scale in SCALES:
                response = ipc("output", output_name, "scale", scale)
                assert all(item.get("success") for item in response), response
                output = next(o for o in ipc("-t", "get_outputs") if o["name"] == output_name)
                assert output["scale"] == scale, output
                # The output's logical size: what xdg-output says, and what
                # the window must measure (Sway's is whole points, 533 at 1.5).
                logical = (output["rect"]["width"], output["rect"]["height"])
                assert logical == (int(MODE[0] / scale), int(MODE[1] / scale)), logical
                name = str(scale).replace(".", "_")
                protocol_path = args.output / f"protocol-{name}.log"
                with protocol_path.open("w") as protocol, (args.output / f"app-{name}.log").open("w") as log:
                    app = subprocess.Popen([str(args.binary.resolve())], env=client_env, stdout=log,
                                           stderr=protocol)
                    deadline = time.monotonic() + 30
                    window = None
                    # Mapped, and drawn at its size: the viewport's destination
                    # set (zgx's frames go through the viewporter).
                    while True:
                        window = window_of(app.pid)
                        drawn = re.search(r"set_destination\(", protocol_path.read_text())
                        if window and window.get("visible") and drawn:
                            break
                        if time.monotonic() > deadline or app.poll() is not None:
                            raise RuntimeError(f"the covering window did not show at scale {scale}; "
                                               f"inspect {protocol_path.name}")
                        time.sleep(.1)
                    time.sleep(.5)
                    window = window_of(app.pid)
                    shot = args.output / f"covers-{name}.png"
                    subprocess.run(["grim", "-o", output_name, str(shot.resolve())], env=env, check=True)
                    subprocess.run(["swaymsg", "-s", str(sockets[0]), f"[pid={app.pid}] kill"], env=env,
                                   capture_output=True)
                    try:
                        app.wait(timeout=10)
                    except subprocess.TimeoutExpired:
                        stop(app)
                protocol = protocol_path.read_text()
                # The first size it asks for, before its surface's first
                # commit: a window that can't be resized asks for its size as
                # its maximum (and minimum), after winit's unbounded one.
                # (libwayland names objects `name#id` from 1.23, `name@id` before.)
                surface = re.search(r"get_xdg_surface\(new id xdg_surface[#@]\d+, wl_surface[#@](\d+)\)", protocol)
                commit = surface and re.search(rf"-> wl_surface[#@]{surface[1]}\.commit\(\)", protocol)
                assert commit, f"no commit of the window's surface in {protocol_path.name}"
                asked = [(int(w), int(h)) for w, h in re.findall(
                    r"-> xdg_toplevel[#@]\d+\.set_max_size\((\d+), (\d+)\)", protocol[:commit.start()])]
                asked = [size for size in asked if size != (0, 0)]
                destinations = [(int(w), int(h)) for w, h in re.findall(r"set_destination\((\d+), (\d+)\)", protocol)]
                rect = window["rect"]
                stage = {"scale": scale, "output_logical": logical, "asked_before_first_commit": asked,
                         "viewport_destinations": destinations[-3:],
                         "window_rect": [rect["x"], rect["y"], rect["width"], rect["height"]],
                         "floating": window.get("type") == "floating_con"}
                stages.append(stage)
                assert asked and asked[0] == logical, ("first size asked", stage)
                assert destinations and destinations[-1] == logical, ("drawn at", stage)
                assert (rect["width"], rect["height"]) == logical and (rect["x"], rect["y"]) == (0, 0), \
                    ("shown at", stage)
            (args.output / "results.json").write_text(json.dumps({
                "passed": True,
                "compositor": subprocess.check_output(["sway", "--version"], text=True).strip(),
                "backend": "native Wayland / owned Sway X11 / Xvfb / pixman",
                "mode": MODE, "stages": stages}, indent=2) + "\n")
            print("Covering-window checks passed")
        finally:
            stop(app)
            stop(compositor)
            stop(wm)
            stop(xvfb)


if __name__ == "__main__":
    main()
