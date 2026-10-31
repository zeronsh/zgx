#!/usr/bin/env python3
"""Text dragged into zgx from another app on Wayland (`wayland_data.rs`):
under a headless sway, a GTK window of the probe's own
(`text_drag_source.py`, GTK's Wayland drag and drop) drags text, with a
virtual pointer (`wayland_pointer.py`), onto drag_probe's Words field. The
display is the probe's own, so nothing else can take a drop.

Checks that:

- hovering over the field changes nothing; let go there, the text lands at
  the drop point (the field's end) and the source hears it taken;
- let go over the chip (no destination there), nothing is dropped and the
  source hears it refused."""
import argparse
import json
import os
import pathlib
import re
import subprocess
import sys
import tempfile
import time

sys.path.insert(0, str(pathlib.Path(__file__).parent))
from platform_smoke import stop  # noqa: E402
from wayland_pointer import VirtualPointer  # noqa: E402
from sway_host import sway_config

OUTPUT = (1600, 1000)
# In drag_probe's Drag window (at the output's top-left): the chip's centre,
# and the Words field's far end (its left 10, 240 wide; 104 down, 30 high).
CHIP = (10 + 60, 10 + 14)
FIELD_END = (10 + 230, 104 + 15)
# The source's button, its window 240 by 120 at (0, 600).
SOURCE = (120, 660)
START = "drag these words"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binaries", type=pathlib.Path, help="the zgx-desktop examples' directory")
    parser.add_argument("--output", type=pathlib.Path, required=True)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    processes, checks = [], {}

    def launch(name, command, env):
        log = args.output / (name + ".log")
        processes.append(subprocess.Popen(list(map(str, command)), env=env, stdout=log.open("w"),
                                          stderr=subprocess.STDOUT))
        return processes[-1], log

    def until(what, test, seconds=15):
        deadline = time.monotonic() + seconds
        while True:
            value = test()
            if value:
                return value
            if time.monotonic() > deadline:
                raise AssertionError(f"timed out waiting for {what}: {checks}")
            time.sleep(.05)

    try:
        with tempfile.TemporaryDirectory(prefix="zgx-text-drop-") as private:
            os.chmod(private, 0o700)
            env = dict(os.environ, XDG_RUNTIME_DIR=private, WLR_BACKENDS="headless", WLR_RENDERER="pixman",
                       WLR_LIBINPUT_NO_DEVICES="1", GDK_BACKEND="wayland", NO_AT_BRIDGE="1")
            for key in ("DISPLAY", "WAYLAND_DISPLAY", "DBUS_SESSION_BUS_ADDRESS"):
                env.pop(key, None)
            config = pathlib.Path(private, "sway.conf")
            sway_config(config,
                "output * mode %dx%d\ndefault_border none\nfor_window [app_id=\".*\"] floating enable\n"
                "for_window [title=\"Drag\"] move position 0 0\n"
                "for_window [title=\"Other\"] move position 900 0\n"
                "for_window [title=\"Text source\"] move position 0 600\n"
                "for_window [title=\"Solo\"] move position 900 600\n" % OUTPUT)
            launch("sway", ["sway", "--unsupported-gpu", "-c", config], env)
            sockets = lambda: [p for p in pathlib.Path(private).glob("wayland-*") if p.suffix != ".lock"]
            display = until("sway's socket", sockets, 10)[0].name
            env["WAYLAND_DISPLAY"] = display
            app, app_log = launch("drag_probe", [args.binaries / "drag_probe"], env)
            _, source_log = launch("source", ["python3", "scripts/text_drag_source.py", "Text source", 0, 0,
                                              " and more"], env)
            until("the source", lambda: "READY" in source_log.read_text())

            def words():
                return re.findall(r'^WORDS "(.*)"$', app_log.read_text(), re.M)

            def lines(pattern):
                return len(re.findall(pattern, source_log.read_text(), re.M))

            until("the field's first value", words)
            time.sleep(1)
            pointer = VirtualPointer(private, display)

            def glide(start, end, steps=15):
                for step in range(1, steps + 1):
                    pointer.move_to(start[0] + (end[0] - start[0]) * step // steps,
                                    start[1] + (end[1] - start[1]) * step // steps, *OUTPUT)
                    time.sleep(.03)

            def grab():
                """Press on the source's button and pull it past GTK's threshold."""
                pointer.move_to(*SOURCE, *OUTPUT)
                time.sleep(.2)
                pointer.button(True)
                time.sleep(.05)
                glide(SOURCE, (SOURCE[0] + 20, SOURCE[1] - 10), 4)
                return (SOURCE[0] + 20, SOURCE[1] - 10)

            # Over the chip, then the field: let go there, it drops at the end.
            at = grab()
            glide(at, CHIP)
            time.sleep(.3)
            glide(CHIP, FIELD_END)
            time.sleep(.4)
            checks["hovering changes nothing"] = words() == [START]
            pointer.button(False)
            until("the drop", lambda: len(words()) > 1)
            until("the drag's end", lambda: lines(r"^END$") == 1)
            checks["dropped"] = words()[-1]
            checks["the text lands at the field's end"] = words()[-1] == START + " and more"
            sent = re.search(r"^SENT (\S+)$", source_log.read_text(), re.M)
            checks["asked for UTF-8 text"] = sent.group(1) if sent else None
            checks["the source hears it taken"] = lines(r"^FAILED") == 0

            # Let go over the chip: refused.
            time.sleep(.5)
            at = grab()
            glide(at, CHIP)
            time.sleep(.4)
            pointer.button(False)
            until("the drag's end", lambda: lines(r"^END$") == 2)
            time.sleep(.3)
            checks["over the chip: nothing dropped"] = len(words()) == 2
            checks["over the chip: refused"] = lines(r"^FAILED") == 1
            checks["app alive"] = app.poll() is None

            (args.output / "result.json").write_text(json.dumps(checks, indent=2) + "\n")
            failed = [key for key, value in checks.items() if value is False or value is None]
            assert not failed, f"failed: {failed}\n{json.dumps(checks, indent=2)}"
            print("PASS: Wayland text drops into zgx: a GTK source's text lands where it was let go over a "
                  "field; over the chip it is refused")
    finally:
        for process in reversed(processes):
            stop(process)


if __name__ == "__main__":
    main()
