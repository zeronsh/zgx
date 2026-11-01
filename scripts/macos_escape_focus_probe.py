#!/usr/bin/env python3
"""Escape closing a menu shown in a popup window of its own, on macOS: opened
with nothing focused, nothing is focused after it (not the window's first
button); opened after a button was clicked, that button has focus back,
without a ring; opened and moved through from the keyboard, its trigger has
focus with its ring.

    python3 scripts/macos_escape_focus_probe.py [--target-dir DIR]

Builds the escape_focus_probe example and runs it (see its header): it posts
AppKit events to its own windows only. The user's pointer is never moved and
nothing outside the process is touched.

It is launched as a direct child, from /tmp, with a clean environment. If a
system dialog appears, stop and report it; never click it. macOS only.
"""
import argparse
import os
import pathlib
import platform
import shutil
import subprocess
import sys
import tempfile

ROOT = pathlib.Path(__file__).resolve().parents[1]


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--target-dir', default=os.environ.get('CARGO_TARGET_DIR', str(ROOT / 'target')))
    args = parser.parse_args()
    if platform.system() != 'Darwin':
        print('macOS only')
        return 2
    env = dict(os.environ, CARGO_TARGET_DIR=args.target_dir)
    subprocess.run(['cargo', 'build', '-q', '-p', 'zgx-desktop', '--example', 'escape_focus_probe', '--locked'],
                   cwd=ROOT, env=env, check=True)
    with tempfile.TemporaryDirectory(prefix='zgx-escape-focus-') as stage:
        binary = pathlib.Path(stage) / 'escape_focus_probe'
        shutil.copy2(pathlib.Path(args.target_dir) / 'debug/examples/escape_focus_probe', binary)
        clean = {key: os.environ[key] for key in ('HOME', 'TMPDIR') if key in os.environ}
        clean['PATH'] = '/usr/bin:/bin'
        out = subprocess.run([str(binary)], cwd='/tmp', env=clean, capture_output=True, text=True,
                             timeout=60).stdout
    print(out, end='')
    lines = out.splitlines()
    line = lambda label: next((l for l in lines if l.startswith(label + ' ')), '')
    checks = [
        ('the menu shows in a popup window', line('OPEN_BLANK').startswith('OPEN_BLANK open=true popups=1 ')),
        ('opened with nothing focused, Escape leaves nothing focused',
         line('ESCAPE_BLANK') == 'ESCAPE_BLANK open=false popups=0 focused=none ring=false'),
        ('a clicked button has focus back, without a ring',
         line('ESCAPE_CLICKED') == 'ESCAPE_CLICKED open=false popups=0 focused=other ring=false'),
        ('opened from the keyboard, an arrow moves in it',
         line('KEYBOARD_OPEN').startswith('KEYBOARD_OPEN open=true popups=1 ')),
        ('closed with Escape, its trigger has focus with its ring',
         line('ESCAPE_KEYBOARD') == 'ESCAPE_KEYBOARD open=false popups=0 focused=anchor ring=true'),
    ]
    failed = [name for name, ok in checks if not ok]
    for name, ok in checks:
        print(('PASS ' if ok else 'FAIL ') + name)
    return 1 if failed else 0


if __name__ == '__main__':
    sys.exit(main())
