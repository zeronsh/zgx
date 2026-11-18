#!/usr/bin/env python3
"""A shortcut acts before the key typed right after it, on an owned Xvfb/Openbox desktop.

shortcut_order's Ctrl+G (a menu item's key, which the window's keymap takes
on Linux) moves the caret to the other field. Twenty times, Ctrl+G and a letter
go to the X server with no gap between them, so the window has both queued
when it reads the first: each letter must land in the field Ctrl+G moved to.
"""
import argparse
import json
import os
import pathlib
import select
import subprocess
import time

from platform_smoke import stop, wait_for_wm

LETTERS = "abcdefghijklmnopqrst"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=pathlib.Path)
    parser.add_argument("--output", type=pathlib.Path, required=True)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    xvfb = wm = app = None
    try:
        with (args.output / "xvfb.log").open("w") as log:
            xvfb = subprocess.Popen(["Xvfb", "-r", "-noreset", "-displayfd", "1", "-screen", "0", "1024x768x24", "-ac"], stdout=subprocess.PIPE, stderr=log, text=True)
            if not select.select([xvfb.stdout], [], [], 30)[0]:
                raise RuntimeError("Xvfb startup timed out")
            display = ":" + xvfb.stdout.readline().strip()
            if display == ":":
                raise RuntimeError("Xvfb startup failed")
        env = dict(os.environ, DISPLAY=display, WINIT_X11_SCALE_FACTOR="1", XDG_RUNTIME_DIR="/tmp", DBUS_SESSION_BUS_ADDRESS="unix:path=/nonexistent")
        env.pop("WAYLAND_DISPLAY", None)
        with (args.output / "openbox.log").open("w") as log:
            wm = subprocess.Popen(["openbox"], env=env, stdout=log, stderr=log)
        wait_for_wm(env)
        app_log = args.output / "application.log"
        with app_log.open("w") as log:
            app = subprocess.Popen([str(args.binary.resolve())], env=env, stdout=log, stderr=subprocess.STDOUT)

        def output():
            return app_log.read_text()

        def wait(token, count=1):
            deadline = time.monotonic() + 10
            while output().count(token) < count:
                if time.monotonic() > deadline or app.poll() is not None:
                    raise RuntimeError(f"missing {token!r} x{count}:\n{output()}")
                time.sleep(.03)

        def xdo(*args):
            return subprocess.check_output(["xdotool", *map(str, args)], env=env, text=True).strip()

        wait("READY")
        deadline = time.monotonic() + 20
        window = None
        while time.monotonic() < deadline and app.poll() is None:
            found = subprocess.run(["xdotool", "search", "--onlyvisible", "--pid", str(app.pid)], env=env, capture_output=True, text=True)
            if found.returncode == 0 and found.stdout.strip():
                window = found.stdout.splitlines()[0]
                break
            time.sleep(.05)
        if window is None:
            raise RuntimeError("shortcut_order window did not open")
        xdo("windowactivate", "--sync", window)
        time.sleep(.3)
        for round, letter in enumerate(LETTERS):
            # One request each, no delay between the keys: both are queued
            # before the window reads the first.
            xdo("key", "--clearmodifiers", "--delay", "0", "ctrl+g", letter)
            wait("OTHER ", round + 1)
            time.sleep(.1)
        xdo("key", "--clearmodifiers", "ctrl+r")
        wait("FIELDS ")
        # The caret starts in the first field; the first Ctrl+G moves it to
        # the second, so the letters of even rounds are the second's.
        expected = 'FIELDS first="{}" second="{}"'.format(LETTERS[1::2], LETTERS[0::2])
        report = next(line for line in output().splitlines() if line.startswith("FIELDS "))
        if report != expected:
            raise AssertionError(f"expected {expected!r}, got {report!r}")
        xdo("windowactivate", "--sync", window, "key", "alt+F4")
        app.wait(timeout=10)
        result = {"platform": "X11/Xvfb", "result": "pass", "rounds": len(LETTERS), "checks": ["a key queued behind a keymap shortcut reaches the field the shortcut focused"]}
        (args.output / "result.json").write_text(json.dumps(result, indent=2) + "\n")
        print("PASS: " + ", ".join(result["checks"]))
    finally:
        stop(app)
        stop(wm)
        stop(xvfb)


if __name__ == "__main__":
    main()
