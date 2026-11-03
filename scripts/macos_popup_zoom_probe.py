#!/usr/bin/env python3
"""Popup windows under a zoomed window on macOS: placed, sized and hit at the zoom.

    python3 scripts/macos_popup_zoom_probe.py [--target-dir DIR]

Builds the popup_zoom_probe example and runs it with its window at 90% and
at 125%. A menu taller than the window opens in a popup window, which must
sit where the scene puts the menu times the zoom, against the content's
top-left, and be that size (within a point). A click posted to its second
item, in the app's own queue, must reach that item.

It is launched as a direct child, from /tmp, with a clean environment; the
only input is that click, posted to the app itself. If a system dialog
appears, stop and report it; never click it. macOS only.
"""
import argparse
import os
import pathlib
import re
import shutil
import subprocess
import sys
import tempfile

ROOT = pathlib.Path(__file__).resolve().parent.parent


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--target-dir', default=os.environ.get('CARGO_TARGET_DIR', str(ROOT / 'target')))
    args = parser.parse_args()
    if sys.platform != 'darwin':
        print('macOS only')
        return 0
    env = dict(os.environ, CARGO_TARGET_DIR=args.target_dir)
    subprocess.check_call(['cargo', 'build', '-q', '-p', 'zgx-desktop', '--example', 'popup_zoom_probe',
                           '--locked'], cwd=ROOT, env=env)
    checks = {}
    with tempfile.TemporaryDirectory(prefix='zgx-popup-zoom-') as stage:
        binary = pathlib.Path(stage) / 'popup_zoom_probe'
        shutil.copy2(pathlib.Path(args.target_dir) / 'debug/examples/popup_zoom_probe', binary)
        clean = {key: os.environ[key] for key in ('HOME', 'TMPDIR', 'USER') if key in os.environ}
        clean['PATH'] = '/usr/bin:/bin'
        for zoom in ('0.9', '1.25'):
            out = subprocess.run([str(binary), '--zoom', zoom], cwd='/tmp', env=clean, capture_output=True,
                                 text=True, timeout=60).stdout
            print(f'== zoom {zoom}')
            print('\n'.join(line for line in out.splitlines() if re.match(r'(PLACED|CLICKED|NO_)', line)))
            placed = re.search(r'^PLACED got=([\d.,-]+) want=([\d.,-]+)$', out, re.M)
            checks[f'{zoom}: done'] = 'POPUP_ZOOM_DONE' in out and placed is not None
            if placed:
                got = [float(v) for v in placed.group(1).split(',')]
                want = [float(v) for v in placed.group(2).split(',')]
                checks[f'{zoom}: the popup window is where the menu is, at the zoom'] = all(
                    abs(g - w) <= 1.0 for g, w in zip(got[:2], want[:2]))
                checks[f'{zoom}: and its size, at the zoom'] = all(
                    abs(g - w) <= 1.0 for g, w in zip(got[2:], want[2:]))
            checks[f'{zoom}: a click on its second item reaches it'] = re.search(
                r'^CLICKED Second$', out, re.M) is not None
    for name, ok in checks.items():
        print(('ok  ' if ok else 'FAIL') + ' ' + name)
    return 0 if checks and all(checks.values()) else 1


if __name__ == '__main__':
    sys.exit(main())
