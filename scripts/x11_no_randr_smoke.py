#!/usr/bin/env python3
"""An X server without RandR (`Xvfb -extension RANDR`), driving the
popup_probe example under Openbox: the app starts (winit asked the server
for RandR's version and panicked without it), its window shows, and its
menu past the window's bottom edge opens in a window of its own, placed on
the one monitor the root window then is, inside the screen."""
import argparse
import json
import os
import pathlib
import re
import select
import subprocess
import time

from platform_smoke import stop, wait_for_wm

SCREEN = (1280, 1024)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=pathlib.Path, help="the popup_probe example")
    parser.add_argument("--output", type=pathlib.Path, required=True)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    processes, checks = [], {}

    def launch(name, command, env, **options):
        log = (args.output / (name + ".log")).open("w")
        options.setdefault("stdout", log)
        child = subprocess.Popen(command, env=env, stderr=log, **options)
        processes.append(child)
        return child

    try:
        env = dict(os.environ, WINIT_X11_SCALE_FACTOR="1", XDG_RUNTIME_DIR="/tmp",
                   DBUS_SESSION_BUS_ADDRESS="unix:path=/nonexistent")
        env.pop("WAYLAND_DISPLAY", None)
        xvfb = launch("xvfb", ["Xvfb", "-r", "-noreset", "-displayfd", "1", "-screen", "0", "%dx%dx24" % SCREEN,
                               "-ac", "-extension", "RANDR"], env, stdout=subprocess.PIPE, text=True)
        if not select.select([xvfb.stdout], [], [], 30)[0]:
            raise RuntimeError("Xvfb startup timed out")
        env["DISPLAY"] = ":" + xvfb.stdout.readline().strip()

        def run(*argv):
            return subprocess.run(list(map(str, argv)), env=env, capture_output=True, text=True).stdout

        checks["no RandR on the server"] = "RANDR" not in run("xdpyinfo", "-queryExtensions")
        assert checks["no RandR on the server"], "the server has RandR"
        launch("openbox", ["openbox"], env)
        wait_for_wm(env)
        # Its output in a file of its own (its errors in the application's log).
        output = args.output / "output.log"
        app = launch("application", [str(args.binary.resolve())], env, stdout=output.open("w"))

        def until(what, test, seconds=15):
            deadline = time.monotonic() + seconds
            while True:
                value = test()
                if value:
                    return value
                if time.monotonic() > deadline or app.poll() is not None:
                    errors = (args.output / "application.log").read_text()
                    raise AssertionError(f"timed out waiting for {what}: {checks}\n{output.read_text()}\n{errors}")
                time.sleep(.05)

        def windows():
            return run("xdotool", "search", "--onlyvisible", "--pid", app.pid).split()

        def geometry(window):
            info = run("xwininfo", "-id", window)
            value = lambda name: int(re.search(name + r":\s+(-?\d+)", info).group(1))
            return (value("Absolute upper-left X"), value("Absolute upper-left Y"),
                    value("Width"), value("Height"))

        until("READY", lambda: "READY" in output.read_text())
        main_window = until("the window", lambda: (windows() or [None])[0])
        checks["started"] = True
        run("xdotool", "windowactivate", "--sync", main_window)
        x, y, w, h = geometry(main_window)
        checks["window"] = [x, y, w, h]
        # The menu hangs past the window's bottom edge: a window of its own,
        # on the screen the root window is.
        run("xdotool", "mousemove", x + 76, y + h - 30)
        time.sleep(.3)
        run("xdotool", "click", 1)
        until("the menu open", lambda: any(line.startswith("OPEN true") for line in output.read_text().splitlines()))
        popup = until("the menu's window", lambda: next((win for win in windows() if win != main_window), None))
        px, py, pw, ph = geometry(popup)
        checks["menu window"] = [px, py, pw, ph]
        checks["past the window"] = py + ph > y + h
        checks["inside the screen"] = px >= 0 and py >= 0 and px + pw <= SCREEN[0] and py + ph <= SCREEN[1]
        (args.output / "result.json").write_text(json.dumps(checks, indent=2) + "\n")
        assert all(value is not False for value in checks.values()), checks
        print("PASS: with no RandR the app starts, and its menu past the window opens in its own window "
              "inside the screen the root window is")
    finally:
        for process in reversed(processes):
            stop(process)


if __name__ == "__main__":
    main()
