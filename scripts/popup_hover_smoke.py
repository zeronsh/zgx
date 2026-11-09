#!/usr/bin/env python3
"""Hovering a window at the back, on X11 and on Wayland (sway): popup_hover's
window with another app's in front over its lower part, the real pointer moved
onto each target. Hovering must never raise or focus the window or open a popup
over the app in front; the app still hears it, and what it wanted shows as the
app comes forward. A tooltip still shows in a window of the app that isn't
active."""
import argparse
import json
import os
import pathlib
import re
import subprocess
import sys
import tempfile
import time

from platform_smoke import stop, wait_for_wm
from sway_host import sway_config
from wayland_pointer import VirtualPointer

TARGETS = ("tip", "card", "row", "overlay")
OUTPUT = (1280, 960)


def at(log, name, seconds=20):
    """`AT name x y` from the log: where to hover, in the window's points."""
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        found = re.search(rf"^AT {name} (\S+) (\S+)$", log.read_text(errors="replace"), re.M)
        if found and "READY" in log.read_text(errors="replace"):
            return float(found.group(1)), float(found.group(2))
        time.sleep(.05)
    raise AssertionError(f"no AT {name}: {log.read_text(errors='replace')[-2000:]}")


def said(log, since):
    lines = log.read_text(errors="replace").splitlines()[since:]
    return [re.sub(r"^PRESENT \S+ ", "", l) for l in lines if l.startswith("HOVER") or "popup show" in l or "popup configured" in l]


def x11(binary, output, target):
    """One target on an owned Xvfb and Openbox."""
    processes = []
    env = dict(os.environ, WINIT_X11_SCALE_FACTOR="1", ZGX_PRESENT_LOG="1", XDG_RUNTIME_DIR=tempfile.mkdtemp(prefix="zgx-hover-"))
    env.pop("WAYLAND_DISPLAY", None)
    try:
        xvfb = subprocess.Popen(["Xvfb", "-r", "-noreset", "-displayfd", "1", "-screen", "0", "1280x960x24", "-ac"],
                                env=env, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True)
        processes.append(xvfb)
        env["DISPLAY"] = ":" + xvfb.stdout.readline().strip()
        processes.append(subprocess.Popen(["openbox"], env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL))
        wait_for_wm(env)
        run = lambda *c: subprocess.run(c, env=env, capture_output=True, text=True).stdout.strip()

        def launch(name):
            log = output / f"x11-{target}-{name}.log"
            app = subprocess.Popen([str(binary)], env=env, stdout=log.open("w"), stderr=subprocess.STDOUT)
            processes.append(app)
            deadline = time.monotonic() + 20
            while time.monotonic() < deadline:
                # The window manager's own: its popups are windows of the pid too.
                for window in re.findall(r"0x[0-9a-f]+", run("xprop", "-root", "_NET_CLIENT_LIST")):
                    if run("xprop", "-id", window, "_NET_WM_PID").endswith(f"= {app.pid}"):
                        return str(int(window, 16)), log
                time.sleep(.05)
            raise AssertionError(f"{name}'s window did not show")

        back, log = launch("back")
        x, y = at(log, target)
        run("xdotool", "windowmove", "--sync", back, "100", "100")
        info = run("xwininfo", "-id", back)
        origin = [int(re.search(rf"Absolute upper-left {a}:\s+(-?\d+)", info).group(1)) for a in "XY"]
        pointer = (origin[0] + round(x), origin[1] + round(y))
        front, _ = launch("front")
        run("xdotool", "windowmove", "--sync", front, str(origin[0] - 20), str(pointer[1] + 24))
        run("xdotool", "windowactivate", "--sync", front)
        time.sleep(.5)

        def top_level(window):
            """The child of the root holding `window`: its frame, under a window manager."""
            while True:
                tree = run("xwininfo", "-id", str(window), "-tree")
                parent = re.search(r"Parent window id: (0x[0-9a-f]+)( \(the root window\))?", tree)
                if parent.group(2):
                    return int(window)
                window = int(parent.group(1), 16)

        def read():
            # The root's children, topmost first.
            lines = run("xwininfo", "-root", "-children").splitlines()
            order = [int(m.group(1), 16) for m in (re.match(r"\s+(0x[0-9a-f]+)", l) for l in lines) if m]
            at = {window: order.index(top_level(window)) for window in (int(front), int(back))}
            shown = []
            for line in lines:
                m = re.match(r"\s+(0x[0-9a-f]+) .*?(\d+)x(\d+)\+\S+\s+\+(-?\d+)\+(-?\d+)", line)
                if not m or int(m.group(2)) <= 1 or "IsViewable" not in run("xwininfo", "-id", m.group(1)):
                    continue
                if run("xprop", "-id", m.group(1), "_NET_WM_WINDOW_TYPE").endswith("_NET_WM_WINDOW_TYPE_POPUP_MENU"):
                    shown.append({"size": [int(m.group(2)), int(m.group(3))], "at": [int(m.group(4)), int(m.group(5))],
                                  "over front": order.index(int(m.group(1), 16)) < at[int(front)]})
            active = int(run("xdotool", "getactivewindow") or "0")
            return {"active is front": active == int(front), "active is back": active == int(back),
                    "back above front": at[int(back)] < at[int(front)], "popups": shown}
        before = read()
        since = len(log.read_text(errors="replace").splitlines())
        run("xdotool", "mousemove", str(pointer[0] - 3), str(pointer[1]))
        run("xdotool", "mousemove", str(pointer[0]), str(pointer[1]))
        time.sleep(1.5)
        after = read()
        heard = said(log, since)
        # Forward, the pointer where it was: what was wanted shows.
        run("xdotool", "windowactivate", "--sync", back)
        time.sleep(1.5)
        forward = read()
        return {"before": before, "after": after, "said": heard, "forward": forward}
    finally:
        for process in reversed(processes):
            stop(process)


def x11_second_window(binary, output):
    """Its other window active: a tooltip in the one that isn't shows, the app
    being forward, and raises nothing over the active one."""
    processes = []
    env = dict(os.environ, WINIT_X11_SCALE_FACTOR="1", POPUP_HOVER_SECOND="1", POPUP_HOVER_AT="100,100",
               ZGX_PRESENT_LOG="1", XDG_RUNTIME_DIR=tempfile.mkdtemp(prefix="zgx-hover-"))
    env.pop("WAYLAND_DISPLAY", None)
    try:
        xvfb = subprocess.Popen(["Xvfb", "-r", "-noreset", "-displayfd", "1", "-screen", "0", "1280x960x24", "-ac"],
                                env=env, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True)
        processes.append(xvfb)
        env["DISPLAY"] = ":" + xvfb.stdout.readline().strip()
        processes.append(subprocess.Popen(["openbox"], env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL))
        wait_for_wm(env)
        run = lambda *c: subprocess.run(c, env=env, capture_output=True, text=True).stdout.strip()
        log = output / "x11-second.log"
        app = subprocess.Popen([str(binary)], env=env, stdout=log.open("w"), stderr=subprocess.STDOUT)
        processes.append(app)
        x, y = at(log, "tip")
        deadline = time.monotonic() + 20
        while len(ids := re.findall(r"0x[0-9a-f]+", run("xprop", "-root", "_NET_CLIENT_LIST"))) < 2:
            assert time.monotonic() < deadline, "its two windows did not show"
            time.sleep(.05)
        first, second = ids[0], ids[1]
        run("xdotool", "windowactivate", "--sync", second)
        time.sleep(.5)
        info = run("xwininfo", "-id", first)
        origin = [int(re.search(rf"Absolute upper-left {a}:\s+(-?\d+)", info).group(1)) for a in "XY"]
        pointer = (origin[0] + round(x), origin[1] + round(y))
        run("xdotool", "mousemove", str(pointer[0] - 3), str(pointer[1]))
        run("xdotool", "mousemove", str(pointer[0]), str(pointer[1]))
        time.sleep(1.5)
        return {"said": said(log, 0), "active is second": int(run("xdotool", "getactivewindow") or "0") == int(second, 16)}
    finally:
        for process in reversed(processes):
            stop(process)


def wayland(binary, output, target):
    """One target on an owned headless sway: focus doesn't follow the pointer."""
    processes = []
    with tempfile.TemporaryDirectory(prefix="zgx-hover-") as private:
        os.chmod(private, 0o700)
        # On X11, for a keyboard on its seat: winit hears focus by the keyboard.
        env = dict(os.environ, XDG_RUNTIME_DIR=private, ZGX_PRESENT_LOG="1", WINIT_UNIX_BACKEND="wayland",
                   WLR_BACKENDS="x11", WLR_X11_OUTPUTS="1", WLR_RENDERER="pixman", WLR_LIBINPUT_NO_DEVICES="1")
        env.pop("WAYLAND_DISPLAY", None)
        config = pathlib.Path(private, "sway.conf")
        sway_config(config, "xwayland disable\nseat seat0 fallback true\noutput * mode %dx%d\ndefault_border none\n"
                            "focus_follows_mouse no\nfor_window [app_id=\".*\"] floating enable\n" % OUTPUT)
        try:
            xvfb = subprocess.Popen(["Xvfb", "-r", "-noreset", "-displayfd", "1", "-screen", "0", "1400x1100x24", "-ac"],
                                    env=env, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True)
            processes.append(xvfb)
            env["DISPLAY"] = ":" + xvfb.stdout.readline().strip()
            processes.append(subprocess.Popen(["sway", "--unsupported-gpu", "-c", str(config)], env=env,
                                              stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL))
            deadline = time.monotonic() + 15
            while not (sockets := [p for p in pathlib.Path(private).glob("wayland-*") if p.suffix != ".lock"]):
                assert time.monotonic() < deadline, "no sway socket"
                time.sleep(.05)
            display = sockets[0].name
            env["WAYLAND_DISPLAY"] = display
            env.pop("DISPLAY")
            env["SWAYSOCK"] = next(pathlib.Path(private).glob("sway-ipc.*"), pathlib.Path("")).as_posix()
            msg = lambda *c: subprocess.run(["swaymsg", *c], env=env, capture_output=True, text=True).stdout

            def launch(name):
                log = output / f"wayland-{target}-{name}.log"
                app = subprocess.Popen([str(binary)], env=env, stdout=log.open("w"), stderr=subprocess.STDOUT)
                processes.append(app)
                return app, log

            def node(pid):
                def find(n):
                    if n.get("pid") == pid:
                        return n
                    for child in n.get("nodes", []) + n.get("floating_nodes", []):
                        if (hit := find(child)):
                            return hit
                return find(json.loads(msg("-t", "get_tree")))

            back, log = launch("back")
            x, y = at(log, target)
            msg(f"[pid={back.pid}]", "move", "position", "100", "100")
            front, _ = launch("front")
            time.sleep(1)
            msg(f"[pid={front.pid}]", "move", "position", "80", str(100 + round(y) + 24))
            msg(f"[pid={front.pid}]", "focus")
            time.sleep(.3)
            read = lambda: {"front focused": bool(node(front.pid)["focused"]), "back focused": bool(node(back.pid)["focused"])}
            before = read()
            since = len(log.read_text(errors="replace").splitlines())
            rect = node(back.pid)["rect"]
            pointer = VirtualPointer(private, display)
            pointer.move_to(rect["x"] + round(x) - 3, rect["y"] + round(y), *OUTPUT)
            time.sleep(.05)
            pointer.move_to(rect["x"] + round(x), rect["y"] + round(y), *OUTPUT)
            time.sleep(1.5)
            shot = output / f"wayland-{target}.png"
            subprocess.run(["grim", str(shot)], env=env, check=False)
            after, heard = read(), said(log, since)
            # Forward, the pointer where it was: what was wanted shows.
            msg(f"[pid={back.pid}]", "focus")
            time.sleep(1.5)
            forward = said(log, since)
            pointer.close()
            return {"before": before, "after": after, "said": heard, "forward said": forward}
        finally:
            for process in reversed(processes):
                stop(process)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=pathlib.Path)
    parser.add_argument("--output", type=pathlib.Path, required=True)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    binary = args.binary.resolve()
    result = {}
    for target in TARGETS:
        result[f"x11 {target}"] = x11(binary, args.output, target)
        result[f"wayland {target}"] = wayland(binary, args.output, target)
    (args.output / "result.json").write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps(result, indent=1))
    # Hovering at the back never focuses or raises the window, opens nothing,
    # and the app still hears it; a popup wanted then shows as it comes forward.
    wrong = []
    for name, read in result.items():
        after = read["after"]
        if after.get("active is back") or after.get("back above front") or after.get("back focused"):
            wrong.append(f"{name}: focused or raised")
        if after.get("popups") or any("popup show" in line for line in read["said"]):
            wrong.append(f"{name}: a popup showed at the back: {read['said']}")
        if not any(line.startswith("HOVER") for line in read["said"]):
            wrong.append(f"{name}: hover not heard at the back")
        target = name.split()[-1]
        if target != "overlay":
            forward = read.get("forward", {}).get("popups") if "forward" in read else any(
                "popup show" in line for line in read["forward said"])
            if not forward:
                wrong.append(f"{name}: nothing showed as it came forward")
    second = x11_second_window(binary, args.output)
    result["x11 second window"] = second
    (args.output / "result.json").write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps(second, indent=1))
    if not second["active is second"]:
        wrong.append("second window lost its focus")
    if not any("popup show" in line for line in second["said"]):
        wrong.append("a tooltip in its window that isn't active didn't show")
    assert not wrong, wrong
    print("PASS: at the back a hover is heard, opens nothing, raises and focuses nothing; "
          "what was wanted shows as the app comes forward, and in its other window")


if __name__ == "__main__":
    sys.exit(main())
