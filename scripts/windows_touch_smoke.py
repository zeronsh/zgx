#!/usr/bin/env python3
"""Touch and pen on Windows, through synthetic touch (InjectTouchInput) and a
synthetic pen (InjectSyntheticPointerInput): no touchscreen needed. Windows
gives touch and pen to the window as pointer messages, not a mouse's, so
this checks what zgx makes of them:

- a tap clicks (the gallery's Increment);
- a finger dragged along a slider drags it;
- a finger dragged over a list pans it, and a flick coasts on after the
  lift and comes to rest;
- a finger held, then lifted, opens the context menu (an editor's);
- a finger held on selectable text selects the word, a drag extends it by
  words, and the lift opens the selection's menu; held on a button in the
  text, it selects and does not press it;
- paging rests on whole pages while its plain neighbours pan and coast independently;
- a pen is a mouse: its stroke over the list does not pan it.

    cargo build -p zgx-desktop --example gallery --example workbench --example form --example selectable_text --example paging
    python scripts/windows_touch_smoke.py target/debug/examples
"""
import argparse
import ctypes
import pathlib
import subprocess
import time
from ctypes import wintypes

import windows_input as win

user32 = ctypes.WinDLL("user32", use_last_error=True)
user32.CreateSyntheticPointerDevice.restype = wintypes.HANDLE
user32.InjectSyntheticPointerInput.argtypes = [wintypes.HANDLE, ctypes.c_void_p, wintypes.UINT]

FLAG_INRANGE, FLAG_INCONTACT = 0x2, 0x4
FLAG_DOWN, FLAG_UPDATE, FLAG_UP = 0x10000, 0x20000, 0x40000
PT_TOUCH, PT_PEN = 2, 3
PHASE = {"down": FLAG_DOWN | FLAG_INRANGE | FLAG_INCONTACT,
         "move": FLAG_UPDATE | FLAG_INRANGE | FLAG_INCONTACT,
         "up": FLAG_UP}


class POINTER_INFO(ctypes.Structure):
    _fields_ = [("pointerType", wintypes.DWORD), ("pointerId", wintypes.UINT), ("frameId", wintypes.UINT),
                ("pointerFlags", wintypes.DWORD), ("sourceDevice", wintypes.HANDLE),
                ("hwndTarget", wintypes.HWND), ("ptPixelLocation", wintypes.POINT),
                ("ptHimetricLocation", wintypes.POINT), ("ptPixelLocationRaw", wintypes.POINT),
                ("ptHimetricLocationRaw", wintypes.POINT), ("dwTime", wintypes.DWORD),
                ("historyCount", wintypes.UINT), ("InputData", ctypes.c_int32),
                ("dwKeyStates", wintypes.DWORD), ("PerformanceCount", ctypes.c_uint64),
                ("ButtonChangeType", ctypes.c_int)]


class POINTER_TOUCH_INFO(ctypes.Structure):
    _fields_ = [("pointerInfo", POINTER_INFO), ("touchFlags", wintypes.DWORD), ("touchMask", wintypes.DWORD),
                ("rcContact", wintypes.RECT), ("rcContactRaw", wintypes.RECT), ("orientation", wintypes.UINT),
                ("pressure", wintypes.UINT)]


class POINTER_PEN_INFO(ctypes.Structure):
    _fields_ = [("pointerInfo", POINTER_INFO), ("penFlags", wintypes.DWORD), ("penMask", wintypes.DWORD),
                ("pressure", wintypes.UINT), ("rotation", wintypes.UINT), ("tiltX", ctypes.c_int32),
                ("tiltY", ctypes.c_int32)]


class POINTER_TYPE_INFO(ctypes.Structure):
    class _Info(ctypes.Union):
        _fields_ = [("touch", POINTER_TOUCH_INFO), ("pen", POINTER_PEN_INFO)]
    _fields_ = [("type", wintypes.DWORD), ("info", _Info)]


def touch(window, contacts):
    """One frame of contacts: [(id, client x, client y, "down" | "move" | "up")]."""
    win.require_foreground(window)
    frame = (POINTER_TOUCH_INFO * len(contacts))()
    for slot, (pointer, x, y, phase) in zip(frame, contacts):
        sx, sy = win.to_screen(window, x, y)
        slot.pointerInfo.pointerType = PT_TOUCH
        slot.pointerInfo.pointerId = pointer
        slot.pointerInfo.pointerFlags = PHASE[phase]
        slot.pointerInfo.ptPixelLocation = wintypes.POINT(sx, sy)
        slot.touchMask = 0x7  # contact area, orientation, pressure
        slot.rcContact = wintypes.RECT(sx - 2, sy - 2, sx + 2, sy + 2)
        slot.orientation, slot.pressure = 90, 32000
    if not user32.InjectTouchInput(len(contacts), frame):
        raise ctypes.WinError(ctypes.get_last_error())


def slide(window, start, end, steps, delay, inject=None):
    inject = inject or (lambda x, y, phase: touch(window, [(0, x, y, phase)]))
    inject(*start, "down")
    for step in range(1, steps + 1):
        time.sleep(delay)
        inject(start[0] + (end[0] - start[0]) * step // steps, start[1] + (end[1] - start[1]) * step // steps, "move")
    time.sleep(delay)
    inject(*end, "up")


def hold(window, x, y, seconds):
    touch(window, [(0, x, y, "down")])
    # Injected contacts time out unless they are updated.
    for _ in range(int(seconds / 0.05)):
        time.sleep(0.05)
        touch(window, [(0, x, y, "move")])
    touch(window, [(0, x, y, "up")])


def region(capture, left, top, right, bottom):
    """A sample of a capture's pixels in client rectangle (left, top)-(right, bottom)."""
    return [win.pixel(capture, x, y) for y in range(top, bottom, 3) for x in range(left, right, 3)]


def launch(examples, name):
    proc = subprocess.Popen([str(examples / f"{name}.exe")], stdout=subprocess.DEVNULL)
    window = win.wait_window(proc)
    time.sleep(1.5)
    win.activate(window)
    return proc, window


def finish(proc, window):
    win.close(window)
    try:
        proc.wait(timeout=10)
    finally:
        if proc.poll() is None:
            proc.kill()


def check(passed, what):
    if not passed:
        raise AssertionError(what)
    print(f"ok: {what}")


PAGE_COLOURS = [(192, 48, 48), (48, 160, 64), (48, 80, 192), (192, 160, 32)]


def paging_cases(examples, plain_only=False):
    """Read paging and plain-scroll offsets from the paging example's colours."""
    import hashlib
    import json
    import statistics

    failures = []

    def expect(passed, case, **values):
        print(json.dumps(dict(case=case, passed=bool(passed), **values)), flush=True)
        if not passed:
            failures.append(case)

    proc, window = launch(examples, "paging")
    try:
        check(win.dpi(window) == 96, "paging pixel probe requires 96 DPI")
        check(win.client_size(window) == (900, 300), "paging has its expected client size")

        def page_pixels(capture):
            return [win.pixel(capture, x, y) for x, y in ((3, 120), (150, 150), (296, 120))]

        def view_hash(capture, axis):
            width, _, pixels = capture
            left = 300 if axis == "down" else 600
            rows = [pixels[(y * width + left + 10) * 4:(y * width + left + 280) * 4]
                    for y in range(10, 280)]
            return hashlib.sha256(b"".join(rows)).hexdigest()

        def offset(capture, axis):
            samples = []
            boundaries = []
            previous = None
            for position in range(3, 290):
                x, y = (450, position) if axis == "down" else (600 + position, 150)
                r, g, b = win.pixel(capture, x, y)
                index = (r - 40) // 5
                if r == g and b == 255 - r and (r - 40) % 5 == 0 and 0 <= index < 40:
                    if previous is not None and index == previous + 1:
                        samples.append(index * 40 - position)
                        boundaries.append([position, index])
                    previous = index
            check(bool(samples), f"{axis}: stripe boundaries can be decoded")
            return statistics.median(samples), boundaries

        def gesture(start, end, steps, delay, hold_before_up=0, sample=None):
            touch(window, [(0, *start, "down")])
            began = time.monotonic()
            for step in range(1, steps + 1):
                time.sleep(max(0, began + step * delay - time.monotonic()))
                x = start[0] + (end[0] - start[0]) * step / steps
                y = start[1] + (end[1] - start[1]) * step / steps
                touch(window, [(0, x, y, "move")])
            duration = time.monotonic() - began
            during = sample(win.capture(window)) if sample else None
            if hold_before_up:
                deadline = time.monotonic() + hold_before_up
                while time.monotonic() < deadline:
                    time.sleep(0.025)
                    touch(window, [(0, *end, "move")])
            else:
                time.sleep(0.001)
            touch(window, [(0, *end, "up")])
            lifted = time.monotonic()
            print(json.dumps(dict(gesture=dict(start=start, end=end, steps=steps,
                requested_delay=delay, moving_seconds=duration,
                speed=((end[0] - start[0]) ** 2 + (end[1] - start[1]) ** 2) ** 0.5 / duration,
                hold_before_up=hold_before_up, during=during))), flush=True)
            return lifted, during

        def after_lift(lifted, seconds):
            time.sleep(max(0, lifted + seconds - time.monotonic()))
            capture = win.capture(window)
            return capture, time.monotonic() - lifted

        if not plain_only:
            actions = [
                ("slow-small", (230, 150), (190, 150), 20, 0.01, 0.15, 0),
                ("flick-next", (230, 150), (180, 150), 5, 0.008, 0, 1),
                ("slow-past-half", (230, 150), (60, 150), 85, 0.01, 0.15, 2),
                ("flick-back", (80, 150), (130, 150), 5, 0.008, 0, 1),
                ("fast-flick-one-page", (230, 150), (130, 150), 5, 0.002, 0, 2),
            ]
            for name, start, end, steps, delay, hold_for, expected in actions:
                before_pixels = page_pixels(win.capture(window))
                lifted, during = gesture(start, end, steps, delay, hold_for, page_pixels)
                expect(during != before_pixels, "paged/" + name + "/follows-finger",
                       before=before_pixels, during=during)
                first, first_at = after_lift(lifted, 1.5)
                second, second_at = after_lift(lifted, 2.5)
                wanted = [PAGE_COLOURS[expected]] * 3
                actual_first = page_pixels(first)
                actual_second = page_pixels(second)
                expect(actual_first == wanted and actual_second == wanted, "paged/" + name,
                    expected_page=expected, pixels_1500=actual_first, pixels_2500=actual_second,
                    sample_times=[first_at, second_at], during=during)

        paged_before = page_pixels(win.capture(window))
        for axis in ("down", "across"):
            start = (450, 230) if axis == "down" else (830, 150)
            pan_end = (450, 170) if axis == "down" else (770, 150)
            flick_end = (450, 150) if axis == "down" else (750, 150)
            before, _ = offset(win.capture(window), axis)
            lifted, during = gesture(start, pan_end, 30, 0.01, 0.15,
                                      lambda capture: offset(capture, axis)[0])
            capture, _ = after_lift(lifted, 0.3)
            panned, pan_boundaries = offset(capture, axis)
            expect(40 <= during - before <= 70 and abs(panned - during) <= 3,
                   axis + "/pan-follows-finger", before=before, during=during,
                   settled=panned, boundaries=pan_boundaries)
            lifted, during = gesture(start, flick_end, 8, 0.008, sample=lambda capture: offset(capture, axis)[0])
            values = {}
            page_samples = [page_pixels(capture)]
            for milliseconds in (80, 500, 2500, 2900):
                capture, actual_at = after_lift(lifted, milliseconds / 1000)
                value, boundaries = offset(capture, axis)
                values[str(milliseconds)] = dict(offset=value, actual_seconds=actual_at, boundaries=boundaries,
                                                  pixel_hash=view_hash(capture, axis))
                page_samples.append(page_pixels(capture))
            early = values["80"]["offset"]
            late = values["500"]["offset"]
            settled = values["2500"]["offset"]
            last = values["2900"]["offset"]
            expect(during > panned + 50, axis + "/flick-follows-finger", before=panned, during=during)
            expect(late > early + 3, axis + "/coasts-after-lift", samples=values)
            expect(abs(last - settled) <= 1 and values["2500"]["pixel_hash"] == values["2900"]["pixel_hash"],
                   axis + "/comes-to-rest", samples=values)
            expect(min(settled % 300, 300 - settled % 300) > 3,
                   axis + "/rests-off-page-multiple", offset=settled, modulo_300=settled % 300)
            expect(all(pixels == paged_before for pixels in page_samples), axis + "/paged-stayed",
                   paged_before=paged_before, samples=page_samples)
    finally:
        finish(proc, window)
    check(not failures, "paging checks passed: " + ", ".join(failures))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("examples", type=pathlib.Path)
    parser.add_argument("--paging-only", action="store_true")
    parser.add_argument("--plain-only", action="store_true")
    parser.add_argument("--existing-only", action="store_true")
    args = parser.parse_args()
    examples = args.examples
    if not user32.InitializeTouchInjection(10, 1):
        raise ctypes.WinError(ctypes.get_last_error())

    if args.paging_only or args.plain_only:
        paging_cases(examples, plain_only=args.plain_only)
        return

    proc, window = launch(examples, "gallery")
    try:
        before = win.capture(window)
        touch(window, [(0, 88, 128, "down")])
        time.sleep(0.05)
        touch(window, [(0, 88, 128, "up")])
        time.sleep(0.5)
        check(region(before, 24, 160, 300, 190) != region(win.capture(window), 24, 160, 300, 190),
              "a tap clicks the gallery's Increment")
        before = win.capture(window)
        slide(window, (190, 313), (330, 313), 12, 0.016)
        time.sleep(0.4)
        check(region(before, 24, 296, 444, 332) != region(win.capture(window), 24, 296, 444, 332),
              "a finger along the slider drags it")
    finally:
        finish(proc, window)

    proc, window = launch(examples, "workbench")
    try:
        library = (44, 190, 290, 300)
        before = win.capture(window)
        slide(window, (160, 700), (160, 400), 20, 0.016)
        time.sleep(0.8)
        panned = win.capture(window)
        check(region(before, *library) != region(panned, *library), "a finger pans the library")
        slide(window, (160, 700), (160, 500), 5, 0.008)
        lifted = win.capture(window)
        time.sleep(0.4)
        coasted = win.capture(window)
        check(region(lifted, *library) != region(coasted, *library), "a flick coasts on after the lift")
        time.sleep(1.5)
        still = win.capture(window)
        time.sleep(0.3)
        check(region(still, *library) == region(win.capture(window), *library), "and comes to rest")
        before = win.capture(window)
        device = user32.CreateSyntheticPointerDevice(PT_PEN, 1, 1)
        if not device:
            raise ctypes.WinError(ctypes.get_last_error())

        def pen(x, y, phase):
            win.require_foreground(window)
            info = POINTER_TYPE_INFO()
            info.type = PT_PEN
            sx, sy = win.to_screen(window, x, y)
            pointer = info.info.pen.pointerInfo
            pointer.pointerType, pointer.pointerId = PT_PEN, 1
            pointer.pointerFlags = PHASE[phase] | (FLAG_INRANGE if phase == "up" else 0)
            pointer.ptPixelLocation = wintypes.POINT(sx, sy)
            info.info.pen.penMask, info.info.pen.pressure = 1, 512
            if not user32.InjectSyntheticPointerInput(device, ctypes.byref(info), 1):
                raise ctypes.WinError(ctypes.get_last_error())

        slide(window, (160, 700), (160, 400), 15, 0.016, inject=pen)
        time.sleep(0.6)
        check(region(before, *library) == region(win.capture(window), *library),
              "a pen's stroke over the list does not pan it")
    finally:
        finish(proc, window)

    proc, window = launch(examples, "form")
    try:
        below = (300, 330, 460, 420)
        before = win.capture(window)
        hold(window, 230, 240, 0.8)
        time.sleep(0.6)
        check(region(before, *below) != region(win.capture(window), *below),
              "a finger held on the notes editor opens its context menu")
        win.key("escape")
    finally:
        finish(proc, window)

    proc, window = launch(examples, "selectable_text")
    try:
        # Where "Drag", the first word, and "paragraph", the fourth, show now
        # (UI Automation's text pattern), not at fixed points: the hold lands
        # on the first and the drag ends on the fourth, as the example lays
        # them out.
        words = win.text_rects(window, "Drag", "paragraph")
        (l0, t0, r0, b0), (l1, t1, r1, b1) = words["Drag"], words["paragraph"]
        x0, y0 = (l0 + r0) / 2, (t0 + b0) / 2
        x1, y1 = (l1 + r1) / 2, (t1 + b1) / 2
        before = win.capture(window)
        touch(window, [(0, x0, y0, "down")])
        for _ in range(16):
            time.sleep(0.05)
            touch(window, [(0, x0, y0, "move")])
        for step in range(1, 13):
            time.sleep(0.02)
            touch(window, [(0, x0 + (x1 - x0) * step / 12, y0 + (y1 - y0) * step / 12, "move")])
        touch(window, [(0, x1, y1, "up")])
        time.sleep(0.6)
        check(region(before, 200, 110, 420, 190) != region(win.capture(window), 200, 110, 420, 190),
              "a finger held on text, dragged and lifted opens the selection's menu")
        win.key("escape")
        time.sleep(0.2)
        win.key("ctrl+c")
        time.sleep(0.3)
        copied = subprocess.run(["powershell", "-NoProfile", "-Command", "Get-Clipboard"],
                                capture_output=True, text=True).stdout.strip()
        check(copied == "Drag across this paragraph",
              f"the hold selected the word and the drag extended it by words ({copied!r})")
        # Held on the "Press me" button in the text: its word, and no click.
        win.key("escape")
        before = win.capture(window)
        hold(window, 55, 240, 0.8)
        time.sleep(0.6)
        win.key("escape")
        time.sleep(0.2)
        win.key("ctrl+c")
        time.sleep(0.3)
        copied = subprocess.run(["powershell", "-NoProfile", "-Command", "Get-Clipboard"],
                                capture_output=True, text=True).stdout.strip()
        check(copied == "Press" and region(before, 123, 230, 230, 250) == region(win.capture(window), 123, 230, 230, 250),
              "a finger held on a button in the text selects its word and does not press it")
    finally:
        finish(proc, window)
    if not args.existing_only:
        paging_cases(examples)
    print("touch smoke passed")


if __name__ == "__main__":
    main()
