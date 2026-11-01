#!/usr/bin/env python3
"""macOS: what assistive apps see of a zgx window (scripts/macos_ax_windows.swift).

Runs the fixed_size_window example (minimum and maximum size the same, still
resizable) and asserts its window is in the app's AXWindows with AXSize and
AXPosition settable, so window managers (Raycast, Rectangle) can move it.
Needs accessibility permission for the process running it; without it the
helper says UNTRUSTED and this exits 2 (it never asks for permission).

    cargo build --release -p zgx-desktop --example fixed_size_window
    python3 scripts/macos_ax_windows_probe.py target/release/examples/fixed_size_window
"""
import argparse
import os
import pathlib
import subprocess
import sys
import tempfile
import time


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=pathlib.Path)
    args = parser.parse_args()
    here = pathlib.Path(__file__).resolve().parent
    with tempfile.TemporaryDirectory() as tmp:
        helper = pathlib.Path(tmp) / "macos_ax_windows"
        subprocess.run(["swiftc", "-O", "-o", str(helper), str(here / "macos_ax_windows.swift")], check=True)
        env = dict(os.environ, ZGX_QUIT_AFTER_MS="5000")
        app = subprocess.Popen([str(args.binary)], env=env, stdout=subprocess.PIPE, text=True)
        try:
            out = subprocess.run([str(helper), str(app.pid), "4", "50"], capture_output=True, text=True).stdout
        finally:
            app.wait(timeout=15)
    print(out, end="")
    if "UNTRUSTED" in out:
        sys.exit(2)
    lines = [l for l in out.splitlines() if "ax=1" in l]
    if not lines:
        sys.exit("FAIL: the window never appeared in AXWindows")
    if not all("size:true pos:true" in l for l in lines):
        sys.exit("FAIL: AXSize or AXPosition not settable")
    print("PASS: listed, its size and position settable")


if __name__ == "__main__":
    main()
