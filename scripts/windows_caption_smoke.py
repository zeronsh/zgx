#!/usr/bin/env python3
"""`Titlebar::Inset`'s caption buttons in a real window (`caption_probe`):
what is drawn, what `WM_NCHITTEST` answers (what snap layouts read) and what
`WindowInfo::controls` reports agree, at each zoom, in the app's theme's ink,
light or dark whatever the system's.

    cargo build -p zgx-desktop --example caption_probe
    python scripts/windows_caption_smoke.py target/debug/examples/caption_probe.exe --output target/caption

It sends no input and activates nothing: each case's app runs in a Job Object
of its own (closed with it), the hit tests are messages to its own window, and
the pictures its own window's (`PrintWindow`). Saves `<case>.png`, the buttons
four times over, for a person to look at.
"""
import argparse
import ctypes
import pathlib
import subprocess
import threading
import time
import winreg
from ctypes import wintypes

import windows_input as win

user32 = ctypes.WinDLL("user32")
user32.SendMessageW.restype = ctypes.c_ssize_t
user32.SendMessageW.argtypes = [wintypes.HWND, wintypes.UINT, wintypes.WPARAM, wintypes.LPARAM]
kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
kernel32.CreateJobObjectW.restype = wintypes.HANDLE
kernel32.CreateJobObjectW.argtypes = [wintypes.LPVOID, wintypes.LPCWSTR]
kernel32.SetInformationJobObject.argtypes = [wintypes.HANDLE, ctypes.c_int, wintypes.LPVOID, wintypes.DWORD]
kernel32.AssignProcessToJobObject.argtypes = [wintypes.HANDLE, wintypes.HANDLE]
kernel32.CloseHandle.argtypes = [wintypes.HANDLE]

HTCLOSE, HTMAXBUTTON, HTMINBUTTON = 20, 9, 8
BANDS = {10, 11, 12, 13, 14, 15, 16, 17}  # HTLEFT .. HTBOTTOMRIGHT


class Limits(ctypes.Structure):
    """JOBOBJECT_EXTENDED_LIMIT_INFORMATION."""

    # 64 bytes of basic limits (LimitFlags at 16), then IO counters and sizes.
    _fields_ = [("words", ctypes.c_uint64 * 18)]


def job_for(proc):
    """A Job Object holding `proc`, which ends it as the handle closes."""
    job = kernel32.CreateJobObjectW(None, None)
    limits = Limits()
    # LimitFlags, at offset 16 of the basic limits: KILL_ON_JOB_CLOSE.
    ctypes.c_uint32.from_buffer(limits, 16).value = 0x2000
    if not kernel32.SetInformationJobObject(job, 9, ctypes.byref(limits), ctypes.sizeof(limits)):
        raise OSError(ctypes.get_last_error(), "SetInformationJobObject")
    if not kernel32.AssignProcessToJobObject(job, int(proc._handle)):
        raise OSError(ctypes.get_last_error(), "AssignProcessToJobObject")
    return job


def system_is_light():
    key = winreg.OpenKey(winreg.HKEY_CURRENT_USER, r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize")
    try:
        return winreg.QueryValueEx(key, "AppsUseLightTheme")[0] != 0
    except OSError:
        return True
    finally:
        winreg.CloseKey(key)


def hit(window, x, y):
    sx, sy = win.to_screen(window, x, y)
    return user32.SendMessageW(window, 0x0084, 0, ((sy & 0xFFFF) << 16) | (sx & 0xFFFF))


def spans(window, width, row):
    """Each button's hit-tested pixels along `row`: {code: (first, last)}."""
    found = {}
    for x in range(width):
        code = hit(window, x, row)
        if code in (HTCLOSE, HTMAXBUTTON, HTMINBUTTON):
            first, _ = found.get(code, (x, x))
            found[code] = (first, x)
    return found


def luminance(capture, x, y):
    r, g, b = win.pixel(capture, x, y)
    return 0.2126 * r + 0.7152 * g + 0.0722 * b


def ink_box(capture, span, height, light):
    """The bounds of the glyph's ink inside a button: pixels well off the
    theme's background, the way the theme has them (dark on light). zgx's
    themes' text is about 100-140 on 246 light, 110-150 on 20 dark, resting or
    in a window in the back."""
    left, right = span
    xs, ys = [], []
    for y in range(height):
        for x in range(left, right + 1):
            value = luminance(capture, x, y)
            if (light and value < 170) or (not light and value > 70):
                xs.append(x)
                ys.append(y)
    return (min(xs), min(ys), max(xs), max(ys)) if xs else None


def crop(capture, left, top, right, bottom, times):
    width, _, data = capture
    rows = bytearray()
    for y in range(top, bottom):
        row = bytearray()
        for x in range(left, right):
            row += data[(y * width + x) * 4:(y * width + x) * 4 + 4] * times
        rows += row * times
    return (right - left) * times, (bottom - top) * times, bytes(rows)


def run_case(binary, output, name, theme, zoom, fixed=None, caption_height=None):
    command = [str(binary), "--theme", theme, "--zoom", str(zoom), "--seconds", "60"]
    if fixed:
        command.append(f"--{fixed}")
    if caption_height:
        command += ["--caption-height", str(caption_height)]
    proc = subprocess.Popen(command, stdout=subprocess.PIPE, text=True)
    job = job_for(proc)
    lines = []
    ready = threading.Event()

    def read():
        for line in proc.stdout:
            lines.append(line.strip())
            if line.startswith("READY"):
                ready.set()

    threading.Thread(target=read, daemon=True).start()
    try:
        window = win.wait_window(proc)
        if not ready.wait(20):
            raise AssertionError(f"{name}: the probe never reported ({lines})")
        time.sleep(0.5)  # Its frames settled.
        controls = next(line for line in lines if line.startswith("CONTROLS"))
        x, y, w, h = (float(v) for v in controls.split()[1:])
        dpi = win.dpi(window) / 96
        pixel = dpi * zoom
        width, height = win.client_size(window)
        tall = round(caption_height * pixel) if caption_height else round(32 * dpi)
        wide = round(46 * dpi)
        row = tall // 2
        found = spans(window, width, row)
        count = 2 if fixed else 3
        problems = []

        def expect(ok, what):
            if not ok:
                problems.append(what)

        # Hit-tested: a button's width each, Close at the corner.
        expect(len(found) == count, f"{len(found)} buttons hit-test: {found}")
        expect((HTMAXBUTTON in found) != bool(fixed), f"Maximize, fixed={fixed}: {found}")
        for code, (first, last) in found.items():
            expect(abs(last - first + 1 - wide) <= 1, f"hit {code} is {last - first + 1} wide, not {wide}")
        expect(hit(window, round(width - 8 * dpi - wide // 2), tall - 1) == HTCLOSE, "the strip's last row is not Close")
        expect(hit(window, width - wide // 2, tall + 1) not in found, "below the strip is a button")
        # Reported: the same pixels, in the content's points.
        left = min(first for first, _ in found.values())
        right = max(last for _, last in found.values())
        expect(abs(x * pixel - left) <= 1.5, f"controls at {x * pixel:.1f} px, hit from {left}")
        expect(abs((x + w) * pixel - right - 1) <= 1.5, f"controls to {(x + w) * pixel:.1f} px, hit to {right + 1}")
        expect(abs(h * pixel - tall) <= 1.5, f"controls {h * pixel:.1f} px tall, not {tall}")
        expect(abs(y) < 0.01, f"controls at y {y}")
        # Fixed: nothing resizes.
        if fixed:
            for point in [(0, height // 2), (width - 1, height // 2), (width // 2, height - 1), (0, height - 1)]:
                expect(hit(window, *point) not in BANDS, f"{point} resizes a fixed-size window")
        # Drawn: each glyph's ink, of the window's theme's, centred on the
        # button hit-tested there.
        capture = win.capture_window(window)
        light = theme == "light"
        boxes = {}
        for code, span in found.items():
            box = ink_box(capture, span, tall, light)
            boxes[code] = box
            expect(box is not None, f"no {theme} ink on button {code}: invisible")
            if box:
                side = round(10 * dpi)
                centre = (box[0] + box[2]) / 2
                middle = (span[0] + span[1]) / 2
                expect(abs(centre - middle) <= 1.5, f"glyph {code} centred at {centre}, its button at {middle}")
                expect(abs((box[1] + box[3]) / 2 - (tall - 1) / 2) <= 1.5, f"glyph {code} rows {box[1]}..{box[3]} of {tall}")
                expect(box[2] - box[0] + 1 <= side + 2, f"glyph {code} {box[2] - box[0] + 1} wide, at {side} drawn")
        picture = crop(capture, max(0, left - 16), 0, width, min(height, tall + 16), 4)
        output.mkdir(parents=True, exist_ok=True)
        win.save_png(picture, output / f"{name}.png")
        status = "ok" if not problems else "FAIL"
        print(f"CASE {name} {status} dpi={dpi} zoom={zoom} hit={sorted(found.items())} "
              f"controls_px=({x * pixel:.1f}, {w * pixel:.1f}, {h * pixel:.1f}) ink={boxes}")
        for problem in problems:
            print(f"  {problem}")
        return not problems
    finally:
        proc.terminate()
        proc.wait(10)
        kernel32.CloseHandle(job)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=pathlib.Path)
    parser.add_argument("--output", type=pathlib.Path, default=pathlib.Path("target/caption"))
    args = parser.parse_args()
    binary = args.binary.resolve()
    print(f"SYSTEM {'light' if system_is_light() else 'dark'}")
    cases = [
        dict(name=f"{theme}-{round(zoom * 100)}", theme=theme, zoom=zoom)
        for theme in ("light", "dark")
        for zoom in (1, 1.5)
    ] + [
        dict(name="dark-90", theme="dark", zoom=0.9),
        dict(name="dark-125", theme="dark", zoom=1.25),
        dict(name="fixed", theme="dark", zoom=1, fixed="fixed"),
        dict(name="fixed-limits", theme="light", zoom=1.5, fixed="fixed-limits"),
        dict(name="height-38", theme="dark", zoom=1, caption_height=38),
        dict(name="height-38-zoomed", theme="light", zoom=1.5, caption_height=38),
    ]
    results = [run_case(binary, args.output, **case) for case in cases]
    print(f"CAPTION_DONE ok={all(results)} {sum(results)}/{len(results)}")
    raise SystemExit(0 if all(results) else 1)


if __name__ == "__main__":
    main()
