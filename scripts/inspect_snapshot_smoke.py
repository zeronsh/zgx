#!/usr/bin/env python3
"""`zgx inspect snapshot` from a real window on X11, on the software raster
(a machine with no GPU, as headless ones and CI's are) and on the GPU: a Luau
app on a private display answers, over its socket, with a PNG of a node as
the window last drew it, cut to the node. A red box comes back red, at its
size. With a menu open in a window of its own (past the window's edge,
under Openbox; on Wayland, under a headless sway, an `xdg_popup`), a
picture of its item is the item's, not the window's behind it."""
import argparse
import json
import os
import pathlib
import socket
import subprocess
import tempfile
import time

from platform_smoke import stop, wait_for_wm
from sway_host import sway_config

APP = """
local pick = ui.state("pick", "item 1")
local items = {}
for i = 1, 24 do
    table.insert(items, "item " .. i)
end
return ui.column {
    p = 20,
    gap = 12,
    h = "100%",
    bg = "#203040",
    ui.column { id = "box", w = 100, h = 60, bg = "#ff0000" },
    ui.column { grow = 1 },
    -- Taller than the window: its menu shows past it.
    ui.select { id = "pick", label = "Pick", value = pick, w = 200, options = items },
}
"""


def ask(path, request):
    with socket.socket(socket.AF_UNIX) as client:
        client.settimeout(10)
        client.connect(str(path))
        client.sendall((json.dumps(request) + "\n").encode())
        answer = b""
        while not answer.endswith(b"\n"):
            chunk = client.recv(65536)
            if not chunk:
                break
            answer += chunk
    return json.loads(answer)


def check_app(app, work, renderer, checks):
    """A picture of the box, then of an item of the menu opened past the
    window, from the app's socket: what the window and the menu showed."""
    from PIL import Image

    path = work / f"zgx-inspect-{os.getuid()}" / f"{app.pid}.sock"

    def until(what, test, seconds=60):
        deadline = time.monotonic() + seconds
        while time.monotonic() < deadline:
            value = test()
            if value:
                return value
            assert app.poll() is None, f"{renderer}: the app ended waiting for {what}"
            time.sleep(.25)
        raise AssertionError(f"{renderer}: timed out waiting for {what}: {checks}")

    def picture(target):
        answer = ask(path, {"cmd": "snapshot", "target": target})
        if not answer["ok"]:
            return None
        data = answer["data"]
        return Image.open(data["path"]).convert("RGBA"), (data["width"], data["height"])

    until("the socket", path.exists)
    # The first frame drawn: a picture of the box.
    image, size = until("a picture of the box", lambda: picture("#box"))
    checks[f"{renderer}: box size"] = size
    checks[f"{renderer}: box is its size"] = size == (100, 60)
    middle = image.getpixel((size[0] // 2, size[1] // 2))
    checks[f"{renderer}: box middle"] = middle
    checks[f"{renderer}: box is red"] = middle[0] > 240 and middle[1] < 16 and middle[2] < 16
    corners = {image.getpixel((x, y)) for x in (1, size[0] - 2) for y in (1, size[1] - 2)}
    checks[f"{renderer}: cut to the box"] = corners == {middle}

    # The menu open: a picture of an item in it is the item.
    assert ask(path, {"cmd": "click", "target": "#pick"})["ok"]
    item = 'role:menuitemradio "item 3"'
    until("the menu open", lambda: ask(path, {"cmd": "find", "target": item})["ok"])
    image, size = until("a picture of an item", lambda: picture(item))
    colours = {image.getpixel((x, y)) for x in range(size[0]) for y in range(size[1])}
    checks[f"{renderer}: item size"] = size
    # Its row (the menu's panel) with its label drawn on it: not the
    # window's flat background behind the menu.
    checks[f"{renderer}: item drawn"] = len(colours) > 8 and (0x20, 0x30, 0x40, 255) not in colours
    windows = ask(path, {"cmd": "windows"})["text"]
    checks[f"{renderer}: windows"] = windows


def run_with(probe, work, renderer, checks):
    app_dir = work / f"app-{renderer}"
    app_dir.mkdir()
    (app_dir / "app.luau").write_text(APP)
    log = (work / f"{renderer}.log").open("w")
    xvfb = subprocess.Popen(["Xvfb", "-r", "-noreset", "-displayfd", "1", "-screen", "0", "1280x1024x24", "-ac"],
                            stdout=subprocess.PIPE, stderr=log, text=True)
    wm = app = None
    try:
        display = ":" + xvfb.stdout.readline().strip()
        env = dict(os.environ, DISPLAY=display, TMPDIR=str(work), ZGX_INSPECT_SOCKET="1",
                   ZGX_RENDERER=renderer, WINIT_X11_SCALE_FACTOR="1", XDG_RUNTIME_DIR="/tmp",
                   DBUS_SESSION_BUS_ADDRESS="unix:path=/nonexistent")
        env.pop("WAYLAND_DISPLAY", None)
        wm = subprocess.Popen(["openbox"], env=env, stdout=log, stderr=log)
        wait_for_wm(env)
        app = subprocess.Popen([str(probe), str(app_dir)], env=env, stdout=log, stderr=log)
        check_app(app, work, renderer, checks)
    finally:
        for process in (app, wm, xvfb):
            stop(process)


def run_wayland(probe, work, checks):
    """The same on a headless sway, drawn on the GPU (popups are surfaces of
    their own there only on it): the window floats at its own size, so the
    menu hangs past its bottom edge in an `xdg_popup`."""
    app_dir = work / "app-wayland"
    app_dir.mkdir()
    (app_dir / "app.luau").write_text(APP)
    log = (work / "wayland.log").open("w")
    runtime = work / "runtime-wayland"
    runtime.mkdir(mode=0o700)
    env = dict(os.environ, XDG_RUNTIME_DIR=str(runtime), WLR_BACKENDS="headless", WLR_RENDERER="pixman",
               WLR_LIBINPUT_NO_DEVICES="1", TMPDIR=str(work), ZGX_INSPECT_SOCKET="1", ZGX_RENDERER="gpu",
               DBUS_SESSION_BUS_ADDRESS="unix:path=/nonexistent")
    env.pop("DISPLAY", None)
    config = runtime / "sway.conf"
    sway_config(config, "output * mode 1280x1024\ndefault_border none\n"
                        "for_window [app_id=\".*\"] floating enable, move position 100 100\n")
    sway = subprocess.Popen(["sway", "--unsupported-gpu", "-c", str(config)], env=env, stdout=log, stderr=log)
    app = None
    try:
        deadline = time.monotonic() + 30
        while not [p for p in runtime.glob("wayland-*") if p.suffix != ".lock"]:
            assert sway.poll() is None and time.monotonic() < deadline, "sway did not start"
            time.sleep(.1)
        env["WAYLAND_DISPLAY"] = next(p for p in runtime.glob("wayland-*") if p.suffix != ".lock").name
        app = subprocess.Popen([str(probe), str(app_dir)], env=env, stdout=log, stderr=log)
        check_app(app, work, "wayland", checks)
    finally:
        for process in (app, sway):
            stop(process)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("probe", type=pathlib.Path, help="zgx-run")
    parser.add_argument("--output", type=pathlib.Path, required=True)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    work = pathlib.Path(tempfile.mkdtemp(dir=args.output))
    checks = {}
    try:
        for renderer in ("raster", "gpu"):
            run_with(args.probe, work, renderer, checks)
        run_wayland(args.probe, work, checks)
    finally:
        (args.output / "result.json").write_text(json.dumps(checks, indent=2, default=str) + "\n")
    failed = {name: value for name, value in checks.items() if value is False}
    assert not failed, json.dumps(checks, indent=2, default=str)
    print("PASS: a node's picture from a window on the raster and on the GPU, and from a menu in a "
          "window of its own, on X11 and on Wayland")


if __name__ == "__main__":
    main()
