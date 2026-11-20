#!/usr/bin/env python3
"""A file dialog where no portal answers, on a KDE session (Xvfb/Openbox,
XDG_CURRENT_DESKTOP=KDE, a session bus with nothing on it): zgx runs
kdialog's chooser, before zenity's, which is on the PATH too. A kdialog of
the probe's own stands in for KDE's (CI has no KDE): it records how it was
asked, attached to the app's X11 window, and answers with a path, which
reaches the app. Then, with no answer, cancelling it (exit 1) leaves the app
running with no result."""
import argparse
import json
import os
import pathlib
import select
import shutil
import subprocess
import time

from platform_smoke import stop, wait_for_wm

# kdialog's stand-in: its arguments to a file, then the fixture's path, or a
# cancel (exit 1) when there is none to give.
KDIALOG = """#!/bin/sh
printf '%s\\n' "$@" > "{args}"
if [ -s "{answer}" ]; then cat "{answer}"; exit 0; fi
exit 1
"""

# A session bus that starts no service: no <servicedir>, no standard ones.
BUS_CONFIG = """<!DOCTYPE busconfig PUBLIC "-//freedesktop//DTD D-Bus Bus Configuration 1.0//EN"
 "http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd">
<busconfig>
  <type>session</type>
  <listen>unix:tmpdir=/tmp</listen>
  <auth>EXTERNAL</auth>
  <policy context="default">
    <allow send_destination="*" eavesdrop="true"/>
    <allow eavesdrop="true"/>
    <allow own="*"/>
  </policy>
</busconfig>
"""


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=pathlib.Path, help="the platform_services example")
    parser.add_argument("--output", type=pathlib.Path, required=True)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    out = args.output.resolve()
    result = {}
    processes = []
    fixture = out / "fixture.txt"
    fixture.write_text("chosen through kdialog\n")
    answer = out / "answer.txt"
    recorded = out / "kdialog-args.txt"
    tools = out / "bin"
    tools.mkdir(exist_ok=True)
    kdialog = tools / "kdialog"
    kdialog.write_text(KDIALOG.format(args=recorded, answer=answer))
    kdialog.chmod(0o755)

    def launch(name, command, env, **options):
        log = (args.output / (name + ".log")).open("w")
        child = subprocess.Popen(command, env=env, stdout=options.pop("stdout", log), stderr=log, **options)
        processes.append(child)
        return child

    try:
        env = dict(os.environ, WINIT_X11_SCALE_FACTOR="1", XDG_RUNTIME_DIR="/tmp", XDG_CURRENT_DESKTOP="KDE")
        env.pop("WAYLAND_DISPLAY", None)
        env["PATH"] = f"{tools}:{env.get('PATH', '')}"
        result["zenity_on_path"] = shutil.which("zenity", path=env["PATH"]) is not None
        xvfb = launch("xvfb", ["Xvfb", "-r", "-noreset", "-displayfd", "1", "-screen", "0", "1280x900x24", "-ac"], env,
                      stdout=subprocess.PIPE, text=True)
        if not select.select([xvfb.stdout], [], [], 30)[0]:
            raise RuntimeError("Xvfb startup timed out")
        env["DISPLAY"] = ":" + xvfb.stdout.readline().strip()
        # A session bus with no portal on it, and none to start: with no
        # service directories, nothing is activated on it (the standard
        # session bus would start xdg-desktop-portal, whose GTK backend
        # answers a KDE session's chooser as a last resort).
        config = out / "session.conf"
        config.write_text(BUS_CONFIG)
        bus = launch("dbus", ["dbus-daemon", f"--config-file={config}", "--nofork", "--print-address=1"], env,
                     stdout=subprocess.PIPE, text=True)
        if not select.select([bus.stdout], [], [], 10)[0]:
            raise RuntimeError("dbus-daemon startup timed out")
        env["DBUS_SESSION_BUS_ADDRESS"] = bus.stdout.readline().strip()
        launch("openbox", ["openbox"], env)
        wait_for_wm(env)

        def until(what, test, seconds=20):
            deadline = time.monotonic() + seconds
            while True:
                value = test()
                if value:
                    return value
                if time.monotonic() > deadline:
                    raise AssertionError(f"timed out waiting for {what}: {result}")
                time.sleep(.1)

        # Open: kdialog is asked, attached to the app's window, and its path
        # reaches the app.
        answer.write_text(f"{fixture}\n")
        log = args.output / "application.log"
        app = launch("application", [str(args.binary.resolve()), "--smoke"], env, stdout=log.open("w"))
        until("the chosen path in the app", lambda: str(fixture) in log.read_text())
        asked = recorded.read_text().splitlines()
        result["kdialog_asked"] = "--getopenfilename" in asked
        result["titled"] = asked[:2] == ["--title", "Open fixture"]
        attach = asked.index("--attach") + 1 if "--attach" in asked else None
        result["attached_to_a_window"] = attach is not None and asked[attach].isdigit() and int(asked[attach]) > 0
        result["chosen"] = "DIALOG open Ok(Some(" in log.read_text()
        result["not_zenity"] = subprocess.run(["pgrep", "-x", "zenity"], capture_output=True).returncode != 0
        stop(app)

        # Cancel: kdialog exits 1, the app hears no file and keeps running.
        answer.write_text("")
        recorded.unlink()
        log = args.output / "cancel.log"
        app = launch("cancel", [str(args.binary.resolve()), "--smoke"], env, stdout=log.open("w"))
        until("kdialog asked again", recorded.exists)
        until("the cancel in the app", lambda: "DIALOG open " in log.read_text(), 10)
        result["cancel_is_no_file"] = "DIALOG open Ok(None)" in log.read_text()
        time.sleep(.5)
        result["app_survives_cancel"] = app.poll() is None
        (args.output / "result.json").write_text(json.dumps(result, indent=2) + "\n")
        assert all(result.values()), result
        print("PASS: on KDE with no portal, kdialog's chooser answers before zenity's, attached, and cancels")
    finally:
        for process in reversed(processes):
            stop(process)


if __name__ == "__main__":
    main()
