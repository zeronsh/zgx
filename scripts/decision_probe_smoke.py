#!/usr/bin/env python3
"""A decision control on X11 (Xvfb/Openbox), driving the decision_probe example:
at each step its own report of the popups in the scene against the windows the
X server shows for it, and a real pointer click at the control's button while a
plugin's modal and its overflowing popover are up. `--sealed`: the same at a
sealed page's button, with a plugin's modal over it and one in its own page.
Passes if the probe does."""
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
    parser.add_argument("binary", type=pathlib.Path, help="the decision_probe example")
    parser.add_argument("--output", type=pathlib.Path, required=True)
    parser.add_argument("--sealed", action="store_true", help="against a sealed page (ui.sealed)")
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    processes, checks = [], {}

    def launch(name, command, env, **options):
        log = (args.output / (name + ".log")).open("w")
        child = subprocess.Popen(command, env=env, stdout=options.pop("stdout", log), stderr=log, **options)
        processes.append(child)
        return child

    try:
        env = dict(os.environ, WINIT_X11_SCALE_FACTOR="1", XDG_RUNTIME_DIR="/tmp",
                   DBUS_SESSION_BUS_ADDRESS="unix:path=/nonexistent",
                   DECISION_PROBE_EXTERNAL="1", DECISION_PROBE_DWELL="2500")
        env.pop("WAYLAND_DISPLAY", None)
        xvfb = launch("xvfb", ["Xvfb", "-r", "-noreset", "-displayfd", "1", "-screen", "0", "%dx%dx24" % SCREEN, "-ac"], env,
                      stdout=subprocess.PIPE, text=True)
        if not select.select([xvfb.stdout], [], [], 30)[0]:
            raise RuntimeError("Xvfb startup timed out")
        env["DISPLAY"] = ":" + xvfb.stdout.readline().strip()
        launch("openbox", ["openbox"], env)
        wait_for_wm(env)
        log = args.output / "application.log"
        app = launch("application", [str(args.binary.resolve())] + (["--sealed"] if args.sealed else []), env,
                     stdout=log.open("w"))

        def run(*argv):
            return subprocess.run(list(map(str, argv)), env=env, capture_output=True, text=True).stdout

        def windows():
            return run("xdotool", "search", "--onlyvisible", "--pid", app.pid).split()

        def origin(window):
            info = run("xwininfo", "-id", window)
            value = lambda name: int(re.search(name + r":\s+(-?\d+)", info).group(1))
            return value("Absolute upper-left X"), value("Absolute upper-left Y")

        seen, main_window = 0, None
        deadline = time.monotonic() + 90
        while app.poll() is None and time.monotonic() < deadline:
            lines = log.read_text().splitlines()
            for line in lines[seen:]:
                if line == "READY":
                    main_window = windows()[0]
                    run("xdotool", "windowactivate", "--sync", main_window)
                elif line.startswith("STEP "):
                    step = line.split()[1]
                    scene = int(re.search(r"scene_popups=(\d+)", line).group(1))
                    # A window shows just after the scene says so, or goes.
                    time.sleep(.5)
                    shown = len(windows()) - 1
                    checks[step] = {"report": line, "x_popup_windows": shown}
                    assert shown == scene, f"{step}: the scene has {scene}, the X server shows {shown}"
                elif line.startswith("CLICK "):
                    _, x, y = line.split()
                    left, top = origin(main_window)
                    at = (left + round(float(x)), top + round(float(y)))
                    run("xdotool", "mousemove", *at)
                    time.sleep(.3)
                    run("xdotool", "click", 1)
                    checks.setdefault("clicks", []).append(at)
            seen = len(lines)
            time.sleep(.05)
        text = log.read_text()
        checks["output"] = text
        assert app.poll() == 0 and "RESULT pass" in text, text
        assert checks["plugin-modal"]["x_popup_windows"] == 0
        if args.sealed:
            assert checks["sealed"]["x_popup_windows"] == 0
            assert checks["page-modal"]["x_popup_windows"] == 0
        else:
            assert checks["free"]["x_popup_windows"] >= 1, "no popup window to begin with"
            assert checks["decision"]["x_popup_windows"] == 0
            assert checks["app-popup"]["x_popup_windows"] == 1
    finally:
        (args.output / "result.json").write_text(json.dumps(checks, indent=2))
        for process in reversed(processes):
            stop(process)
    print(json.dumps({key: value for key, value in checks.items() if key != "output"}, indent=2))


if __name__ == "__main__":
    main()
