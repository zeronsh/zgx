#!/usr/bin/env python3
"""Pointer lock, natively: scripts/pointer_lock/app.luau, run by zgx-desktop's
luau_probe example, on X11 (Xvfb/Openbox, xdotool) and as a Wayland client
of a headless sway (virtual pointer), checked from what it prints.

X11 (no lock in the protocol: the pointer is confined to the window):
- a press on the game area locks the pointer (`el:lock_pointer()` in
  `on_press`, `on_pointer_lock(true)`);
- the mouse's motion arrives raw, as `on_pointer_move`'s `dx`, `dy` with
  `locked`, and the pointer stays inside the window however far it moves;
- other keys still reach the focused game's `on_key`;
- Escape lets it go (`on_pointer_lock(false)`), goes to no `on_key`, and
  is no gesture to lock with: the handler's own `lock_pointer` is refused;
- the pointer is back where it locked, and motion no longer comes;
- a later key reaches `on_key` again.

Wayland (sway nested on the Xvfb, with its keyboard and pointer:
`zwp_pointer_constraints_v1` and relative pointer):
- a press locks the pointer (`zwp_locked_pointer_v1`), and it holds;
- relative motion arrives as `dx`, `dy` with `locked`;
- Escape lets it go, reaches no `on_key`, is no gesture to lock with, and
  the locked pointer is destroyed."""
import argparse
import json
import os
import pathlib
import re
import select
import subprocess
import sys
import tempfile
import time

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
from platform_smoke import release_mounts, stop, wait_for_wm  # noqa: E402
from sway_host import sway_config  # noqa: E402

APP = pathlib.Path(__file__).resolve().parent / "pointer_lock" / "app.luau"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("runner", type=pathlib.Path, help="the luau_probe example")
    parser.add_argument("--output", type=pathlib.Path, required=True)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    (args.output / "result.json").unlink(missing_ok=True)
    processes, checks = [], {}

    def launch(name, command, env):
        log = args.output / (name + ".log")
        processes.append(subprocess.Popen(command, env=env, stdout=log.open("w"), stderr=subprocess.STDOUT))
        return processes[-1], log

    def until(what, test, seconds=15):
        deadline = time.monotonic() + seconds
        while True:
            value = test()
            if value:
                return value
            if time.monotonic() > deadline:
                raise AssertionError(f"timed out waiting for {what}: {json.dumps(checks, indent=2)}")
            time.sleep(.05)

    def lines(log, prefix):
        found = re.findall(rf"^{prefix}(?: (.*))?$", log.read_text(errors="replace"), re.M)
        # `LOCK` is the lock's changes; `LOCK asked` what the press asked.
        return [line for line in found if prefix != "LOCK" or not line.startswith("asked")]

    def motion(log):
        found = lines(log, "MOTION")
        return tuple(map(int, found[-1].split())) if found else None

    try:
        with tempfile.TemporaryDirectory(prefix="zgx-pointer-lock-") as private:
            os.chmod(private, 0o700)
            base = dict(os.environ, XDG_RUNTIME_DIR=private, XDG_CONFIG_HOME=f"{private}/config",
                        XDG_DATA_HOME=f"{private}/data", ZGX_LINT="0",
                        DBUS_SESSION_BUS_ADDRESS="unix:path=/nonexistent")
            for key in ("DISPLAY", "WAYLAND_DISPLAY", "SWAYSOCK", "I3SOCK"):
                base.pop(key, None)

            # X11.
            xvfb = subprocess.Popen(["Xvfb", "-r", "-noreset", "-displayfd", "1", "-screen", "0", "1280x900x24",
                                     "-ac"], stdout=subprocess.PIPE, stderr=(args.output / "xvfb.log").open("w"),
                                    text=True)
            processes.append(xvfb)
            if not select.select([xvfb.stdout], [], [], 30)[0]:
                raise RuntimeError("Xvfb startup timed out")
            x11 = dict(base, DISPLAY=":" + xvfb.stdout.readline().strip(), WINIT_X11_SCALE_FACTOR="1")
            launch("openbox", ["openbox"], x11)
            wait_for_wm(x11)

            def xdo(*parts):
                return subprocess.run(["xdotool", *map(str, parts)], env=x11, check=True,
                                      capture_output=True, text=True).stdout.strip()

            def pointer_at():
                found = dict(re.findall(r"(\w+):(-?\d+)", xdo("getmouselocation")))
                return int(found["x"]), int(found["y"])

            app, log = launch("x11-app", [str(args.runner), str(APP)], x11)

            def window():
                found = subprocess.run(["xdotool", "search", "--onlyvisible", "--pid", str(app.pid)], env=x11,
                                       capture_output=True, text=True).stdout.split()
                return found[0] if found else None

            win = until("x11: the window", window, 20)
            xdo("windowactivate", "--sync", win)
            time.sleep(.4)
            info = subprocess.run(["xwininfo", "-id", win], env=x11, capture_output=True, text=True).stdout
            left, top, width, height = (int(re.search(rf"{k}:\s+(-?\d+)", info).group(1))
                                        for k in ("Absolute upper-left X", "Absolute upper-left Y",
                                                  "Width", "Height"))
            xdo("mousemove", "--sync", "--window", win, 200, 150)
            locked_at = pointer_at()
            xdo("click", 1)
            until("x11: the lock", lambda: "true" in lines(log, "LOCK"))
            checks["x11: a press locks the pointer"] = lines(log, "LOCK asked") == ["true"]
            # The mouse's motion, raw: 3 steps of 40 to the right, 2 of 25 down.
            for _ in range(3):
                xdo("mousemove_relative", "--", 40, 0)
                time.sleep(.05)
            for _ in range(2):
                xdo("mousemove_relative", "--", 0, 25)
                time.sleep(.05)
            moved = until("x11: raw motion", lambda: (m := motion(log)) and m[0] >= 100 and m[1] >= 40 and m)
            checks[f"x11: motion arrives raw as dx, dy (summed {moved})"] = True
            # Far past the window's edge: confined, the pointer stays inside it.
            xdo("mousemove_relative", "--", 3000, 2000)
            time.sleep(.2)
            x, y = pointer_at()
            checks[f"x11: the pointer stays in the window ({x}, {y})"] = (
                left <= x < left + width and top <= y < top + height)
            # Other keys still reach the focused game (a game's WASD).
            xdo("key", "w")
            until("x11: a key while locked", lambda: "w" in lines(log, "KEY"))
            checks["x11: other keys reach on_key while locked"] = True
            keys_before = len(lines(log, "KEY"))
            xdo("key", "Escape")
            until("x11: the lock's end", lambda: "false" in lines(log, "LOCK"))
            time.sleep(.3)
            checks["x11: Escape lets it go"] = lines(log, "LOCK")[-1] == "false"
            checks["x11: Escape goes to no on_key"] = len(lines(log, "KEY")) == keys_before
            checks["x11: Escape is no gesture to lock with"] = lines(log, "RELOCK") == ["false"]
            checks[f"x11: the pointer is back where it locked ({pointer_at()} for {locked_at})"] = (
                pointer_at() == locked_at)
            after = motion(log)
            xdo("mousemove_relative", "--", 30, 0)
            time.sleep(.3)
            checks["x11: unlocked, no more raw motion"] = motion(log) == after
            xdo("key", "a")
            until("x11: a key after", lambda: len(lines(log, "KEY")) > keys_before)
            checks["x11: the next key reaches on_key"] = lines(log, "KEY")[-1] == "a"
            stop(app)

            # Wayland: sway nested on the same Xvfb (wlroots' X11 backend), so
            # its seat has the X server's keyboard and pointer, driven by
            # xdotool through sway's window; sway sends the pointer's moves
            # to a locked pointer's client as relative motion.
            sway_env = dict(x11, WLR_BACKENDS="x11", WLR_X11_OUTPUTS="1", WLR_RENDERER="pixman")
            config = pathlib.Path(private, "sway.conf")
            sway_config(config, "xwayland disable\nseat seat0 fallback true\noutput * mode 960x640\n"
                              "default_border none\nfor_window [app_id=\".*\"] fullscreen enable\n")
            sway, _ = launch("sway", ["sway", "--unsupported-gpu", "-c", str(config)], sway_env)
            sockets = lambda: [p for p in pathlib.Path(private).glob("wayland-*") if p.suffix != ".lock"]
            socket = until("sway's socket", sockets, 10)[0].name

            def host_window():
                tree = subprocess.run(["xwininfo", "-root", "-tree"], env=x11, capture_output=True, text=True).stdout
                found = re.search(r'(0x[0-9a-f]+) "wlroots - X11-1".*960x640', tree)
                return found.group(1) if found else None

            host = until("sway's window", host_window, 10)
            xdo("windowactivate", "--sync", host)
            wayland = dict(sway_env, WAYLAND_DISPLAY=socket, WAYLAND_DEBUG="1", WINIT_UNIX_BACKEND="wayland")
            wayland.pop("DISPLAY", None)
            app, log = launch("wayland-app", [str(args.runner), str(APP)], wayland)
            until("wayland: the window", lambda: re.search(r"wl_surface[#@]\d+\.attach\(wl_buffer",
                                                           log.read_text(errors="replace")), 20)
            until("wayland: the keyboard's focus", lambda: re.search(r"wl_keyboard[#@]\d+\.enter\(",
                                                                    log.read_text(errors="replace")), 10)
            time.sleep(.5)
            xdo("mousemove", "--sync", "--window", host, 200, 150)
            time.sleep(.2)
            xdo("click", 1)
            until("wayland: the lock", lambda: "true" in lines(log, "LOCK"))
            time.sleep(.3)
            trace = log.read_text(errors="replace")
            checks["wayland: a press locks the pointer (zwp_locked_pointer_v1)"] = bool(
                re.search(r"zwp_pointer_constraints_v1[#@]\d+\.lock_pointer\(", trace))
            checks["wayland: the lock holds (no refusal)"] = lines(log, "LOCK") == ["true"]
            for _ in range(3):
                xdo("mousemove_relative", "--", 40, 0)
                time.sleep(.05)
            for _ in range(2):
                xdo("mousemove_relative", "--", 0, 25)
                time.sleep(.05)
            moved = until("wayland: relative motion", lambda: (m := motion(log)) and m[0] >= 100 and m[1] >= 40 and m)
            checks[f"wayland: relative motion arrives as dx, dy (summed {moved})"] = True
            keys_before = len(lines(log, "KEY"))
            xdo("key", "Escape")
            until("wayland: the lock's end", lambda: "false" in lines(log, "LOCK"))
            time.sleep(.3)
            trace = log.read_text(errors="replace")
            checks["wayland: Escape lets it go"] = lines(log, "LOCK")[-1] == "false"
            checks["wayland: Escape goes to no on_key"] = len(lines(log, "KEY")) == keys_before
            checks["wayland: Escape is no gesture to lock with"] = lines(log, "RELOCK") == ["false"]
            checks["wayland: the locked pointer is destroyed"] = bool(
                re.search(r"zwp_locked_pointer_v1[#@]\d+\.destroy\(", trace))
            subprocess.run(["swaymsg", "-s", str(next(pathlib.Path(private).glob("sway-ipc*.sock"))),
                            f"[pid={app.pid}] kill"], env=x11, capture_output=True)
            stop(app)
            stop(sway)
            for process in reversed(processes):
                stop(process)
            release_mounts(private)

        (args.output / "result.json").write_text(json.dumps(checks, indent=2) + "\n")
        failed = [key for key, value in checks.items() if value is False]
        assert not failed, f"failed: {failed}\n{json.dumps(checks, indent=2)}"
        print("PASS: a press locks the pointer, its motion arrives raw, other keys still reach it, and Escape "
              "lets it go to where it locked and no further (X11, confined; Wayland, locked)")
    finally:
        for process in reversed(processes):
            stop(process)


if __name__ == "__main__":
    main()
