#!/usr/bin/env python3
"""A double click on a `window_drag` region zooming the window, on macOS: each
size the animation passes through reaches the app while the window has it. Told
only after the animation, the app leaves its last frame stretched over the
window until it ends.

    python3 scripts/macos_double_click_zoom_probe.py [--target-dir DIR]

Builds the double_click_zoom_probe example and runs it (see its header): it
posts an AppKit press to its own window only. The user's pointer is never moved
and nothing outside the process is touched. It needs the system's title bar
double click to zoom or fill (Desktop & Dock, "Double-click a window's title
bar to"), as it does by default.

It is launched as a direct child, from /tmp, with a clean environment. If a
system dialog appears, stop and report it; never click it. macOS only.
"""
import argparse
import os
import pathlib
import platform
import re
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
    subprocess.run(['cargo', 'build', '-q', '-p', 'zgx-desktop', '--example', 'double_click_zoom_probe', '--locked'],
                   cwd=ROOT, env=env, check=True)
    with tempfile.TemporaryDirectory(prefix='zgx-double-click-zoom-') as stage:
        binary = pathlib.Path(stage) / 'double_click_zoom_probe'
        shutil.copy2(pathlib.Path(args.target_dir) / 'debug/examples/double_click_zoom_probe', binary)
        clean = {key: os.environ[key] for key in ('HOME', 'TMPDIR') if key in os.environ}
        clean['PATH'] = '/usr/bin:/bin'
        out = subprocess.run([str(binary)], cwd='/tmp', env=clean, capture_output=True, text=True,
                             timeout=60).stdout
    print(out, end='')
    zoom = re.search(r'^ZOOM steps=(\d+) stale=(\d+) grew=(\w+) ', out, re.M)
    checks = [
        ('the double click zooms the window', bool(zoom) and zoom[3] == 'true'),
        ('the animation reaches the app in steps', bool(zoom) and int(zoom[1]) >= 5),
        ('each step reaches the app while the window has that size', bool(zoom) and zoom[2] == '0'),
    ]
    failed = [name for name, ok in checks if not ok]
    for name, ok in checks:
        print(('PASS ' if ok else 'FAIL ') + name)
    return 1 if failed else 0


if __name__ == '__main__':
    sys.exit(main())
