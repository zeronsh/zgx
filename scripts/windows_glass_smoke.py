#!/usr/bin/env python3
"""Glass windows on Windows 11 (`WindowOptions::glass`): each material asks
the compositor for its system backdrop, read back from outside the app
(`DwmGetWindowAttribute`, `DWMWA_SYSTEMBACKDROP_TYPE`): Acrylic for the
heads-up glass, Mica for a window's background, Mica Alt for a title bar's,
none for plain glass (its blur is the window's accent instead).
Acrylic must also show: Windows draws it while the window is active and a
solid colour once another window is, so an empty patch of the window must
change as a second test window (placed clear of it) takes the activation,
where an opaque fill would stay the same.

With "Transparency effects" off (Personalization, Colors) the compositor
draws the backdrops as solid colours: the noise is not checked then.

    cargo build -p zgx-desktop --example glass_counter
    python scripts/windows_glass_smoke.py target/debug/examples/glass_counter.exe
"""
import argparse
import ctypes
import os
import pathlib
import subprocess
import time
import winreg
from ctypes import wintypes

import windows_input as win

dwmapi = ctypes.WinDLL("dwmapi")
dwmapi.DwmGetWindowAttribute.argtypes = [wintypes.HWND, wintypes.DWORD, ctypes.c_void_p, wintypes.DWORD]
DWMWA_SYSTEMBACKDROP_TYPE = 38
# Material (ZGX_GLASS_MATERIAL), the backdrop it asks for, its name.
MATERIALS = (("hud", 3, "Acrylic"), ("window", 2, "Mica"), ("titlebar", 4, "Mica Alt"),
             ("plain", 1, "none (the accent's blur)"))
# An empty patch of glass_counter's window, in points: left of the count.
PATCH = (16, 120, 56, 160)


def transparency_effects():
    try:
        with winreg.OpenKey(winreg.HKEY_CURRENT_USER,
                            r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize") as key:
            return winreg.QueryValueEx(key, "EnableTransparency")[0] != 0
    except OSError:
        return True


def backdrop(window):
    value = ctypes.c_int(-1)
    dwmapi.DwmGetWindowAttribute(window, DWMWA_SYSTEMBACKDROP_TYPE, ctypes.byref(value), ctypes.sizeof(value))
    return value.value


def patch(window):
    """An empty patch's distinct colours and mean colour."""
    scale = win.dpi(window) / 96
    shot = win.capture(window)
    left, top, right, bottom = (round(v * scale) for v in PATCH)
    pixels = [win.pixel(shot, x, y) for x in range(left, right) for y in range(top, bottom)]
    mean = tuple(sum(p[i] for p in pixels) / len(pixels) for i in range(3))
    return len(set(pixels)), mean


def other_window(binary):
    """A second test window, at the work area's right, made active."""
    proc = subprocess.Popen([str(binary)])
    window = win.wait_window(proc)
    time.sleep(1)
    area = wintypes.RECT()
    ctypes.windll.user32.SystemParametersInfoW(0x0030, 0, ctypes.byref(area), 0)  # SPI_GETWORKAREA
    win.user32.SetWindowPos(window, None, area.right - 360, area.top + 20, 0, 0, 0x1 | 0x4)
    win.activate(window)
    time.sleep(0.6)
    return proc, window


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("binary", type=pathlib.Path)
    args = parser.parse_args()
    effects = transparency_effects()
    results = []
    for material, expected, name in MATERIALS:
        proc = subprocess.Popen([str(args.binary)], env=dict(os.environ, ZGX_GLASS_MATERIAL=material))
        try:
            window = win.wait_window(proc)
            time.sleep(1)
            win.activate(window)
            time.sleep(0.6)
            asked = backdrop(window)
            assert asked == expected, f"{material}: the window asked for backdrop {asked}, not {name} ({expected})"
            if material == "hud" and effects:
                # Clear of the other window's place at the work area's right.
                win.user32.SetWindowPos(window, None, 40, 40, 0, 0, 0x1 | 0x4)
                time.sleep(0.4)
                active = patch(window)
                other, other_hwnd = other_window(args.binary)
                try:
                    inactive = patch(window)
                finally:
                    other.kill()
                    other.wait()
                moved = max(abs(a - b) for a, b in zip(active[1], inactive[1]))
                assert active[0] > inactive[0] + 3 or moved > 3,                     f"Acrylic: the patch is the same active and inactive ({active} / {inactive}): not drawn"
                results.append(f"{name} (active {active[0]} colours, inactive {inactive[0]}, mean moved {moved:.0f})")
            else:
                results.append(name)
            win.close(window)
            proc.wait(timeout=10)
        finally:
            if proc.poll() is None:
                proc.kill()
                proc.wait()
    print("PASS: " + ", ".join(results) + ("" if effects else "; transparency effects off: noise not checked"))


if __name__ == "__main__":
    main()
