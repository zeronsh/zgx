#!/usr/bin/env python3
"""A text area's atoms (chips) through the X11 clipboard (Xvfb/Openbox,
xdotool keys, xclip): the clipboard_atoms example, typed into.

- Ctrl+A, Ctrl+C in its text area "a" puts its atoms beside the text: the
  clipboard's text is the text, and its web custom data
  (`chromium/x-web-custom-data`, where zgx keeps an app's own types)
  holds `application/x-zgx-atoms` with the atom's range and name;
- Ctrl+V into "b" of the same window makes the atom again (the app's own
  copy, as it was);
- Ctrl+V into "b" of a second process's window makes it again from what
  the clipboard holds alone (its codec reading the name back);
- an offer of the text alone from another client pastes no atom.

The atoms type itself crosses the system clipboard as any app type does
on Wayland, macOS and Windows (clipboard_types_smoke.py and CI's
`types_round_trip_through_the_system_clipboard`); here the keys and the
paste are the desktop host's own."""
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

sys.path.insert(0, str(pathlib.Path(__file__).parent))
from platform_smoke import release_mounts, stop, wait_for_wm  # noqa: E402
from clipboard_types_smoke import unpickle  # noqa: E402

# The example's text areas, in its window's points.
A, B = (210, 50), (210, 130)
ATOMS = "application/x-zgx-atoms"
TEXT = "see @file.pdf now"
ATOM = "[4..13:file-1]"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("example", type=pathlib.Path, help="the clipboard_atoms example")
    parser.add_argument("--output", type=pathlib.Path, required=True)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
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

    try:
        with tempfile.TemporaryDirectory(prefix="zgx-clipboard-atoms-") as private:
            os.chmod(private, 0o700)
            env = dict(os.environ, XDG_RUNTIME_DIR=private, DBUS_SESSION_BUS_ADDRESS="unix:path=/nonexistent")
            for key in ("DISPLAY", "WAYLAND_DISPLAY"):
                env.pop(key, None)
            xvfb = subprocess.Popen(["Xvfb", "-r", "-noreset", "-displayfd", "1", "-screen", "0", "1280x900x24",
                                     "-ac"], stdout=subprocess.PIPE, stderr=(args.output / "xvfb.log").open("w"),
                                    text=True)
            processes.append(xvfb)
            if not select.select([xvfb.stdout], [], [], 30)[0]:
                raise RuntimeError("Xvfb startup timed out")
            env["DISPLAY"] = ":" + xvfb.stdout.readline().strip()
            launch("openbox", ["openbox"], env)
            wait_for_wm(env)

            def window(suffix):
                found = subprocess.run(["xdotool", "search", "--onlyvisible", "--name",
                                        f"^zgx clipboard atoms{suffix}$"], env=env, capture_output=True,
                                       text=True).stdout.split()
                return found[0] if found else None

            def click(suffix, at):
                win = until(f"window{suffix}", lambda: window(suffix))
                subprocess.run(["xdotool", "windowactivate", "--sync", win], env=env)
                subprocess.run(["xdotool", "mousemove", "--window", win, str(at[0]), str(at[1]), "click", "1"],
                               env=env, check=True)
                time.sleep(.3)

            def keys(*names):
                for name in names:
                    subprocess.run(["xdotool", "key", "--clearmodifiers", name], env=env, check=True)
                    time.sleep(.15)

            def read(target):
                out = subprocess.run(["xclip", "-selection", "clipboard", "-t", target, "-o"], env=env,
                                     capture_output=True, timeout=10)
                return out.stdout if out.returncode == 0 and out.stdout else None

            def lines(log, field):
                return re.findall(rf"^ATOMS {field} \w+ .*$", log.read_text(), re.M)

            first, first_log = launch("first", [str(args.example.resolve())], env)
            until("a's atom made", lambda: lines(first_log, "a"))
            checks["a at start"] = lines(first_log, "a")[-1]

            # Copy out of "a".
            click("", A)
            keys("ctrl+a", "ctrl+c")
            text = until("the copy's text", lambda: read("UTF8_STRING"))
            blob = until("the copy's types", lambda: read("chromium/x-web-custom-data"))
            types = unpickle(blob) or {}
            checks["copied text"] = text.decode()
            checks["copied atoms"] = types.get(ATOMS)
            checks["the copy's text is the text"] = text.decode() == TEXT
            atoms = json.loads(types.get(ATOMS, "null") or "null") or {}
            checks["the atoms go beside it, with the atom's range and name"] = (
                atoms.get("text") == TEXT
                and atoms.get("atoms") == [{"from": 4, "to": 13, "atom": {"id": "file-1"}}])

            # Into "b" of the same window: the app's own copy.
            click("", B)
            keys("ctrl+v")
            pasted = until("b's paste", lambda: lines(first_log, "b"))[-1]
            checks["pasted in the window"] = pasted
            checks["a paste in the window makes the atom"] = pasted == f"ATOMS b edit {ATOM}"

            # Into "b" of another process: the clipboard alone.
            second, second_log = launch("second", [str(args.example.resolve()), " 2"], env)
            until("the second window", lambda: window(" 2"))
            until("the second window's a", lambda: lines(second_log, "a"))
            click(" 2", B)
            keys("ctrl+v")
            pasted = until("the second b's paste", lambda: lines(second_log, "b"))[-1]
            checks["pasted in another process"] = pasted
            checks["a paste in another process makes the atom from the clipboard"] = (
                pasted == f"ATOMS b edit {ATOM}")

            # The text alone from another client: no atom.
            offer = subprocess.Popen(["xclip", "-selection", "clipboard", "-t", "UTF8_STRING", "-i"], env=env,
                                     stdin=subprocess.PIPE, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            offer.stdin.write(TEXT.encode())
            offer.stdin.close()
            processes.append(offer)
            # (xclip serves its text for any target asked: its offer is in
            # the targets listed, the web custom data gone from them.)
            until("xclip's offer", lambda: "chromium" not in (read("TARGETS") or b"").decode())
            count = len(lines(second_log, "b"))
            click(" 2", B)
            keys("End", "ctrl+v")
            time.sleep(1)
            after = lines(second_log, "b")
            checks["after a plain paste"] = after[-1]
            checks["the text alone pastes no atom"] = (
                len(after) == count or after[-1] == f"ATOMS b edit {ATOM}")
            for process in reversed(processes):
                stop(process)
            release_mounts(private)

        (args.output / "result.json").write_text(json.dumps(checks, indent=2) + "\n")
        failed = [key for key, value in checks.items() if value is False]
        assert not failed, f"failed: {failed}\n{json.dumps(checks, indent=2)}"
        print("PASS: a copy puts a text area's atoms beside its text on the X11 clipboard, a paste makes them "
              "again in the same window and in another process's, and the text alone pastes none")
    finally:
        for process in reversed(processes):
            stop(process)


if __name__ == "__main__":
    main()
