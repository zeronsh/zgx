#!/usr/bin/env python3
"""Validate that a modifier pressed alone reaches Ui::modifiers, and that the
window losing focus clears it, on owned X11."""
import argparse
import hashlib
import json
import os
import pathlib
import select
import subprocess
import time
from platform_smoke import stop, wait_for_wm


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=pathlib.Path)
    parser.add_argument("--output", type=pathlib.Path, required=True)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    binary_sha256 = hashlib.sha256(args.binary.read_bytes()).hexdigest()
    xvfb = wm = app = None
    try:
        with (args.output / "xvfb.log").open("w") as log:
            xvfb = subprocess.Popen(["Xvfb", "-r", "-noreset", "-displayfd", "1", "-screen", "0", "900x650x24", "-ac"], stdout=subprocess.PIPE, stderr=log, text=True)
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
        app = subprocess.Popen([str(args.binary.resolve()), "--smoke-test"], env=env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)

        def xdo(*arguments):
            return subprocess.check_output(["xdotool", *map(str, arguments)], env=env, text=True).strip()

        subprocess.check_call(["xset", "r", "off"], env=env)
        deadline = time.monotonic() + 20
        window = None
        while time.monotonic() < deadline and app.poll() is None:
            found = subprocess.run(["xdotool", "search", "--onlyvisible", "--name", "^zgx held modifiers$"], env=env, capture_output=True, text=True)
            if found.returncode == 0 and found.stdout.strip():
                window = found.stdout.splitlines()[0]
                break
            time.sleep(.05)
        if window is None:
            raise RuntimeError("held-modifiers window did not open")
        xdo("windowactivate", "--sync", window)
        time.sleep(.3)

        def tap(key):
            xdo("keydown", key)
            time.sleep(.15)
            xdo("keyup", key)
            time.sleep(.15)

        # Super (Command's place) alone, then Shift alone.
        tap("Super_L")
        tap("Shift_L")
        # Super held as the window loses focus: cleared there, not left held.
        xdo("keydown", "Super_L")
        time.sleep(.15)
        xdo("windowminimize", "--sync", window)
        time.sleep(.3)
        xdo("keyup", "Super_L")
        time.sleep(.2)
        output, _ = app.communicate(timeout=15)
        (args.output / "held-modifiers.log").write_text(output)
        if app.returncode:
            raise RuntimeError(output)
        seen = [line.split(" ", 1)[1] for line in output.splitlines() if line.startswith("MODIFIERS ")]
        expected = ["none", "meta", "none", "shift", "none", "meta", "none"]
        if seen != expected:
            raise AssertionError(f"modifiers seen {seen}, expected {expected}\n{output}")
        (args.output / "results.json").write_text(json.dumps({"passed": True, "backend": "X11", "binary_sha256": binary_sha256, "script_sha256": hashlib.sha256(pathlib.Path(__file__).read_bytes()).hexdigest(), "checks": ["Super pressed and released alone reaches Ui::modifiers", "Shift pressed and released alone reaches Ui::modifiers", "a modifier held as the window loses focus is cleared"]}, indent=2) + "\n")
        print("Native held-modifier checks passed")
    finally:
        for process in (app, wm, xvfb):
            stop(process)


if __name__ == "__main__":
    main()
