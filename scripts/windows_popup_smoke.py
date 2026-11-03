#!/usr/bin/env python3
"""Popup windows on Windows (`popup_window/win32.rs`), through the popup
probe: a menu hanging past its window's bottom edge, in a window of its own.
The probe posts mouse messages to its own windows only (the user's pointer
and other apps are never touched). Checks that:

- no popup window exists before the first popup (an app that never opens
  one makes none);
- the menu opens past the window's edge (at 100%; at larger scales the
  window is taller and it may flip above its trigger), within a few
  milliseconds of the click once a popup window is ready;
- hovering and clicking its items chooses one, five times over, and a press
  on the trigger dragged to an item and released there chooses it;
- the menu shows on screen (a screenshot of it open differs from one of its
  place once closed);
- it moves with its window, and closes as the app deactivates and on a press
  on the title bar;
- it is where the scene puts it, its size, at the window's scale and zoom
  (`--zoom z` zooms the probe's window; its clicks go where the zoom draws).

`--place-only` checks only the last: the probe opens its menu itself and
reports its window's place, with no input posted and no need to be in front.

`--idle` also waits through the probe's idle spans (about a minute) and
reports its CPU time before any popup and after one closed.

    cargo build -p zgx-desktop --example popup_probe
    python scripts/windows_popup_smoke.py target/debug/examples
"""
import argparse
import os
import pathlib
import re
import subprocess
import sys

import windows_input as win


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("bin", type=pathlib.Path)
    parser.add_argument("--output", type=pathlib.Path, default=pathlib.Path("target/windows-popup"))
    parser.add_argument("--idle", action="store_true")
    parser.add_argument("--max-open-ms", type=float, default=50.0)
    parser.add_argument("--zoom", type=float, help="zoom the probe's window (its popups at that zoom too)")
    parser.add_argument("--place-only", action="store_true", help="only where it is: no input, no activation")
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    env = dict(os.environ, PROBE_SHOT="1")
    if args.zoom:
        env["POPUP_PROBE_ZOOM"] = str(args.zoom)
    if args.place_only:
        return place_only(args, env)
    if not args.idle:
        env["PROBE_QUICK"] = "1"
    proc = subprocess.Popen([str(args.bin / "popup_probe.exe")], env=env, stdout=subprocess.PIPE,
                            text=True, encoding="utf-8", errors="replace")
    lines, rect, shots = [], None, {}
    try:
        window = win.wait_window(proc, timeout=30)
        # Over other windows, so its menu shows over them too.
        win.activate(window)
        for line in proc.stdout:
            line = line.strip()
            lines.append(line)
            print(line, flush=True)
            if line.startswith("POPUP_RECT"):
                left, top, right, bottom = map(int, line.split()[1:5])
                rect = (left, top, right - left, bottom - top)
            elif line in ("SHOT_OPEN", "SHOT_CLOSED") and rect:
                shots[line] = win.capture_rect(*rect)
                name = line[5:].lower()
                win.save_png(shots[line], args.output / f"popup-{name}.png")
                # With what is around it (DWM's rim and shadow), to look at.
                x, y, w, h = rect
                win.save_png(win.capture_rect(x - 24, y - 24, w + 48, h + 48),
                             args.output / f"popup-{name}-around.png")
        proc.wait(timeout=30)
    finally:
        if proc.poll() is None:
            proc.kill()
    out = "\n".join(lines)

    def need(pattern):
        if not re.search(pattern, out, re.M):
            raise AssertionError(f"missing {pattern!r} in:\n{out}")

    need(r"^POPUP_WINDOWS_BEFORE 0$")
    check_placed(out)
    # At 100% the window leaves room below it on the screen; larger, the menu
    # may flip above its trigger instead.
    if re.search(r"^POPUP_RECT .* scale=1$", out, re.M):
        need(r"^POPUP_RECT .* past_window_bottom=true scale=1$")
    else:
        need(r"^POPUP_RECT ")
    for round in range(5):
        need(rf'^ROUND {round} chosen="Item 6" open=false popups=0$')
    need(r'^PRESS_DRAG chosen="Item 3" open=false closed_after_ms=Some')
    need(r"^FOLLOW moved=Some\(\(40, 30\)\)$")
    need(r"^DEACTIVATE shown_before=true open_after=false popups=0$")
    need(r"^TITLEBAR shown_before=true open_after=false popups=0$")
    latencies = [float(ms) for ms in re.search(r"^OPEN_LATENCY_MS (.*)$", out, re.M).group(1).split()]
    # The first click finds a window readied as the pointer reached the trigger.
    if max(latencies) > args.max_open_ms:
        raise AssertionError(f"a menu took {max(latencies):.1f} ms to show: {latencies}")
    if "SHOT_OPEN" not in shots or "SHOT_CLOSED" not in shots:
        raise AssertionError("no screenshots taken")
    width, height, opened = shots["SHOT_OPEN"]
    closed = shots["SHOT_CLOSED"][2]
    changed = sum(1 for i in range(0, len(opened), 4) if opened[i:i + 3] != closed[i:i + 3])
    if changed < width * height // 3:
        raise AssertionError(f"the menu did not show on screen: {changed} of {width * height} pixels differ")
    print(f"PASS: popup windows (open {', '.join(f'{ms:.1f}' for ms in latencies)} ms; "
          f"{100 * changed // (width * height)}% of its box drawn)")


def check_placed(out):
    # Where the scene puts it, its size, times scale and zoom (within a
    # pixel or two: edges round).
    placed = re.search(r"^ZOOM_PLACED zoom=\S+ got=([\d,-]+) want=([\d,.-]+)$", out, re.M)
    if not placed:
        raise AssertionError(f"no ZOOM_PLACED line in:\n{out}")
    got = [float(v) for v in placed.group(1).split(",")]
    want = [float(v) for v in placed.group(2).split(",")]
    if any(abs(g - w) > 2 for g, w in zip(got, want)):
        raise AssertionError(f"the popup window isn't where the scene puts it at its zoom: got {got}, want {want}")
    return got, want


def place_only(args, env):
    env["POPUP_PROBE_PLACE_ONLY"] = "1"
    proc = subprocess.Popen([str(args.bin / "popup_probe.exe")], env=env, stdout=subprocess.PIPE,
                            text=True, encoding="utf-8", errors="replace")
    try:
        out, _ = proc.communicate(timeout=60)
    finally:
        if proc.poll() is None:
            proc.kill()
    print(out, flush=True)
    if not re.search(r"^PLACE_ONLY_DONE$", out, re.M):
        raise AssertionError(f"the probe didn't finish:\n{out}")
    got, want = check_placed(out)
    print(f"PASS: popup window placed at zoom {args.zoom or 1} (got {got}, want {want})")


if __name__ == "__main__":
    sys.exit(main())
