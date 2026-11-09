#!/usr/bin/env python3
"""A scroll view that pages, as a native Wayland client of a headless sway: a
virtual pointer (`wayland_pointer.py`) scrolls with two fingers at 120 Hz and
lifts them (axis stop), the real path of a touchpad's scroll and its coast.

- on the paging view: a slow small swipe goes back to its page; a flick goes on
  to the next, the page's colour filling the view; a slow swipe past half goes
  on; a flick back comes back. It rests on each, whatever the coast sends after.
- beside it, views that don't page move as they always have, in both axes: they
  move with the fingers and coast, and rest off any page's multiple.

A touchscreen can't be injected here (`windows_touch_smoke.py` does that on
Windows); nor can a mouse wheel's notches, which `tests/scroll_paging.rs` drives.
"""
import argparse
import json
import os
import pathlib
import subprocess
import tempfile
import time

from platform_smoke import stop
from wayland_pointer import VirtualPointer
from sway_host import sway_config

OUTPUT = (1280, 800)
PAGE, HEIGHT = 300, 300
# The paging example's pages, in order.
PAGES = [(0xc0, 0x30, 0x30), (0x30, 0xa0, 0x40), (0x30, 0x50, 0xc0), (0xc0, 0xa0, 0x20)]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=pathlib.Path, help="the paging example")
    parser.add_argument("--output", type=pathlib.Path, required=True)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    from PIL import Image, ImageChops
    processes = []

    def launch(name, command, env):
        log = (args.output / (name + ".log")).open("w")
        processes.append(subprocess.Popen(command, env=env, stdout=log, stderr=log))
        return processes[-1]

    with tempfile.TemporaryDirectory(prefix="zgx-paging-") as private:
        os.chmod(private, 0o700)
        env = dict(os.environ, XDG_RUNTIME_DIR=private, WLR_BACKENDS="headless", WLR_RENDERER="pixman",
                   WLR_LIBINPUT_NO_DEVICES="1")
        for key in ("DISPLAY", "WAYLAND_DISPLAY", "DBUS_SESSION_BUS_ADDRESS"):
            env.pop(key, None)
        config = pathlib.Path(private, "sway.conf")
        sway_config(config, "output * mode %dx%d\ndefault_border none\n"
                          "for_window [app_id=\".*\"] floating enable, move position 0 0\n" % OUTPUT)
        try:
            launch("sway", ["sway", "--unsupported-gpu", "-c", str(config)], env)
            deadline = time.monotonic() + 10
            while not [p for p in pathlib.Path(private).glob("wayland-*") if p.suffix != ".lock"]:
                if time.monotonic() > deadline:
                    raise RuntimeError("sway did not start; see sway.log")
                time.sleep(.05)
            display = next(p.name for p in pathlib.Path(private).glob("wayland-*") if p.suffix != ".lock")
            env["WAYLAND_DISPLAY"] = display
            app = launch("application", [str(args.binary.resolve())], env)

            def shot(name):
                path = args.output / f"{name}.png"
                subprocess.check_call(["grim", str(path)], env=env)
                return Image.open(path).convert("RGB")

            def centre_colour(name):
                # Up and left of the middle: the pointer, which a shot shows, rests there.
                return shot(name).getpixel((PAGE // 2 - 40, HEIGHT // 2 - 40))

            def resting_on(page, name):
                # It comes to rest within the spring's time, and stays.
                deadline = time.monotonic() + 3
                while centre_colour(name) != PAGES[page] or \
                        shot(name).getpixel((5, 5)) != PAGES[page] or \
                        shot(name).getpixel((PAGE - 5, HEIGHT - 5)) != PAGES[page]:
                    if time.monotonic() > deadline:
                        raise AssertionError(f"{name}: not at rest on page {page}: "
                                             f"{centre_colour(name)}, see {name}.png")
                    time.sleep(.1)
                time.sleep(.6)
                seen = centre_colour(name)
                assert seen == PAGES[page], f"{name}: left page {page}: {seen}"

            deadline = time.monotonic() + 20
            while centre_colour("start") != PAGES[0]:
                if time.monotonic() > deadline or app.poll() is not None:
                    raise RuntimeError("the paging example did not show")
                time.sleep(.2)

            pointer = VirtualPointer(private, display)
            result = {}
            pointer.move_to(PAGE // 2, HEIGHT // 2, *OUTPUT)
            time.sleep(.3)
            # Well under 300 points a second, whatever a software renderer does to
            # when scrolls arrive: 1 point a frame at 120 Hz, 120 a second, 40 of 300.
            pointer.swipe(dx=1, frames=40)
            resting_on(0, "slow-small")
            result["slow_small_stays"] = True
            # A flick: 6 points a frame, 720 points a second, 48 points along.
            pointer.swipe(dx=6, frames=8)
            resting_on(1, "flick")
            result["flick_goes_on"] = True
            # Slow, past half: 1 point a frame for 170 of 300.
            pointer.swipe(dx=1, frames=170)
            resting_on(2, "slow-past-half")
            result["slow_past_half_goes_on"] = True
            # A flick back.
            pointer.swipe(dx=-6, frames=8)
            resting_on(1, "flick-back")
            result["flick_back_comes_back"] = True

            # The views beside it don't page, in either axis.
            for name, x, swipe in (("down", PAGE + PAGE // 2, {"dy": 6}),
                                   ("across", 2 * PAGE + PAGE // 2, {"dx": 6})):
                box = (x - PAGE // 2, 0, x + PAGE // 2, HEIGHT)
                pointer.move_to(x, HEIGHT // 2, *OUTPUT)
                time.sleep(.3)
                before = shot(f"{name}-before").crop(box)
                pointer.swipe(frames=8, **swipe)
                lifted = time.monotonic()
                time.sleep(max(0., lifted + .08 - time.monotonic()))
                early = shot(f"{name}-80ms").crop(box)
                time.sleep(max(0., lifted + .5 - time.monotonic()))
                late = shot(f"{name}-500ms").crop(box)
                time.sleep(max(0., lifted + 2.5 - time.monotonic()))
                settled = shot(f"{name}-2500ms").crop(box)
                time.sleep(.4)
                after = shot(f"{name}-2900ms").crop(box)
                moved = ImageChops.difference(before, early).getbbox() is not None
                coasted = ImageChops.difference(early, late).getbbox() is not None
                rested = ImageChops.difference(settled, after).getbbox() is None
                result[name] = {"moved": moved, "coasted": coasted, "rested": rested}
                assert moved, f"{name}: the fingers moved nothing"
                assert coasted, f"{name}: it stopped as the fingers lifted"
                assert rested, f"{name}: it never stopped"
            # And the paged view stayed where it was through all that.
            assert centre_colour("end") == PAGES[1], "the paged view moved with the others"
            pointer.close()
            (args.output / "result.json").write_text(json.dumps(result, indent=2) + "\n")
            print("PASS: a scroll view rests on whole pages, and those beside it scroll as ever")
        finally:
            for process in reversed(processes):
                stop(process)


if __name__ == "__main__":
    main()
