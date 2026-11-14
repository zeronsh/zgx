#!/usr/bin/env python3
"""Rich text through the clipboard on X11 (Xvfb/Openbox, xclip) and as a
Wayland client of a headless sway (wl-copy, wl-paste, a virtual pointer):
the clipboard_html example's Paste and Copy, clicked.

- HTML another client put there as `text/html` (a browser's copy, with
  markup, an entity and a non-ASCII letter) pastes as it was offered
  (`Platform::clipboard_html`), its text beside it where offered;
- Copy offers its HTML as `text/html` and its text as plain text
  (`set_clipboard_rich_text`), which the other client reads back exactly;
- an offer of plain text alone pastes no HTML."""
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
from wayland_pointer import VirtualPointer  # noqa: E402
from sway_host import sway_config

# The example's buttons, in its window's points.
PASTE, COPY = (80, 52), (208, 52)
OFFERED = '<meta charset="utf-8"><p>Pasted <b>bold</b> &amp; <a href="https://zgx.dev">a link</a> é</p>'
COPIED_HTML = "<p>Copied <b>bold</b> and <i>italic</i></p>"
COPIED_TEXT = "Copied bold and italic"


def rust_debug(value):
    """`value` as Rust's `{:?}` prints a string."""
    return '"' + value.replace("\\", "\\\\").replace('"', '\\"') + '"'


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("example", type=pathlib.Path, help="the clipboard_html example")
    parser.add_argument("--output", type=pathlib.Path, required=True)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    processes, checks = [], {}

    def launch(name, command, env, **options):
        log = args.output / (name + ".log")
        processes.append(subprocess.Popen(command, env=env, stdout=options.pop("stdout", log.open("w")),
                                          stderr=subprocess.STDOUT, **options))
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

    def session(name, env, offer, read, click):
        """`offer(data, kind)` puts `data` on the clipboard as another client,
        `read(kind)` reads it back as `kind` (or None), `click(x, y)` presses
        in the example's window."""
        offer(OFFERED, "html")
        app, log = launch(f"{name}-app", [str(args.example.resolve())], env)
        pasted = lambda: re.findall(r"^PASTED .*$", log.read_text(), re.M)
        line = until(f"{name}: the paste", lambda: pasted() or click(*PASTE), 20)[-1]
        checks[f"{name} pasted"] = line
        checks[f"{name}: another client's HTML pastes as it was offered"] = line.startswith(
            f"PASTED html=Some({rust_debug(OFFERED)})")
        click(*COPY)
        until(f"{name}: the copy", lambda: re.search(r"^COPIED written=true$", log.read_text(), re.M))
        html = until(f"{name}: the copy's HTML read back", lambda: read("html"))
        text = until(f"{name}: the copy's text read back", lambda: read("text"))
        checks[f"{name} read back"] = [html, text]
        checks[f"{name}: the copy reaches another client as HTML and as text"] = (
            html == COPIED_HTML and text == COPIED_TEXT)
        # Plain text alone: no HTML pastes.
        offer("only text", "text")
        count = len(pasted())
        until(f"{name}: the paste of plain text", lambda: len(pasted()) > count or click(*PASTE))
        checks[f"{name} pasted plain"] = pasted()[-1]
        checks[f"{name}: plain text alone pastes no HTML"] = pasted()[-1] == 'PASTED html=None text=Some("only text")'
        stop(app)

    try:
        with tempfile.TemporaryDirectory(prefix="zgx-clipboard-html-") as private:
            os.chmod(private, 0o700)
            base = dict(os.environ, XDG_RUNTIME_DIR=private, DBUS_SESSION_BUS_ADDRESS="unix:path=/nonexistent")
            for key in ("DISPLAY", "WAYLAND_DISPLAY"):
                base.pop(key, None)

            # X11.
            xvfb = subprocess.Popen(["Xvfb", "-r", "-noreset", "-displayfd", "1", "-screen", "0", "1280x900x24",
                                     "-ac"], stdout=subprocess.PIPE, stderr=(args.output / "xvfb.log").open("w"),
                                    text=True)
            processes.append(xvfb)
            if not select.select([xvfb.stdout], [], [], 30)[0]:
                raise RuntimeError("Xvfb startup timed out")
            x11 = dict(base, DISPLAY=":" + xvfb.stdout.readline().strip())
            launch("openbox", ["openbox"], x11)
            wait_for_wm(x11)
            x11_types = {"html": "text/html", "text": "UTF8_STRING"}

            def x11_offer(data, kind):
                # xclip serves it until another client takes the selection.
                offer = subprocess.Popen(["xclip", "-selection", "clipboard", "-t", x11_types[kind], "-i"], env=x11,
                                         stdin=subprocess.PIPE, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
                offer.stdin.write(data.encode())
                offer.stdin.close()
                processes.append(offer)
                until("xclip's offer", lambda: x11_types[kind] in subprocess.run(
                    ["xclip", "-selection", "clipboard", "-t", "TARGETS", "-o"], env=x11, capture_output=True,
                    text=True).stdout)

            def x11_read(kind):
                read = subprocess.run(["xclip", "-selection", "clipboard", "-t", x11_types[kind], "-o"], env=x11,
                                      capture_output=True, timeout=10)
                return read.stdout.decode() if read.returncode == 0 and read.stdout else None

            def x11_click(x, y):
                found = subprocess.run(["xdotool", "search", "--onlyvisible", "--name", "^zgx clipboard html$"],
                                       env=x11, capture_output=True, text=True).stdout.split()
                if found:
                    subprocess.run(["xdotool", "mousemove", "--window", found[0], str(x), str(y), "click", "1"],
                                   env=x11, check=True)
                time.sleep(.3)

            session("x11", x11, x11_offer, x11_read, x11_click)

            # Wayland: the window at the output's top left.
            sway_env = dict(base, WLR_BACKENDS="headless", WLR_RENDERER="pixman", WLR_LIBINPUT_NO_DEVICES="1")
            config = pathlib.Path(private, "sway.conf")
            sway_config(config, "output * mode 1280x960\ndefault_border none\n"
                              "for_window [app_id=\".*\"] floating enable, move position 0 0\n")
            launch("sway", ["sway", "--unsupported-gpu", "-c", str(config)], sway_env)
            sockets = lambda: [p for p in pathlib.Path(private).glob("wayland-*") if p.suffix != ".lock"]
            socket = until("sway's socket", sockets, 10)[0].name
            wayland = dict(sway_env, WAYLAND_DISPLAY=socket)
            pointer = VirtualPointer(private, socket)
            wayland_types = {"html": "text/html", "text": "text/plain;charset=utf-8"}

            def wayland_offer(data, kind):
                # wl-copy serves it, forked, until the selection is replaced.
                subprocess.run(["wl-copy", "--type", wayland_types[kind]], env=wayland, input=data.encode(),
                               check=True)
                until("wl-copy's offer", lambda: wayland_types[kind] in subprocess.run(
                    ["wl-paste", "--list-types"], env=wayland, capture_output=True, text=True).stdout)

            def wayland_read(kind):
                read = subprocess.run(["wl-paste", "--no-newline", "--type", wayland_types[kind]], env=wayland,
                                      capture_output=True, timeout=10)
                return read.stdout.decode() if read.returncode == 0 and read.stdout else None

            def wayland_click(x, y):
                pointer.move_to(x, y, 1280, 960)
                time.sleep(.2)
                pointer.click()
                time.sleep(.3)

            session("wayland", wayland, wayland_offer, wayland_read, wayland_click)
            pointer.close()
            for process in reversed(processes):
                stop(process)
            release_mounts(private)

        (args.output / "result.json").write_text(json.dumps(checks, indent=2) + "\n")
        failed = [key for key, value in checks.items() if value is False]
        assert not failed, f"failed: {failed}\n{json.dumps(checks, indent=2)}"
        print("PASS: another client's HTML pastes as offered, plain text alone pastes none, and a copy reaches "
              "another client as HTML and as text (X11, Wayland)")
    finally:
        for process in reversed(processes):
            stop(process)


if __name__ == "__main__":
    main()
