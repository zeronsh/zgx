#!/usr/bin/env python3
"""An inset title bar in full screen on macOS: no opaque strip over the top.

    python3 scripts/macos_fullscreen_toolbar_probe.py [--target-dir DIR]

Builds the fullscreen_toolbar_probe example and runs it twice: into and out
of full screen as the green button does (`toggleFullScreen:`), then with
zgx's `set_fullscreen` (winit's borderless full screen). In full screen,
AppKit's toolbar window above it (NSToolbarFullScreenWindow) must show
nothing (alpha 0, or not visible), the inset toolbar must be hidden and the
content layout rect must be the window's whole height. After, the toolbar
and the traffic lights are back and the inset strip leaves the content
layout rect shorter again.

It is launched as a direct child, from /tmp, with a clean environment; it
sends no input. If a system dialog appears, stop and report it; never click
it. macOS only. Full screen takes a Space of its own for a few seconds.
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
    subprocess.check_call(['cargo', 'build', '-q', '-p', 'zgx-desktop', '--example', 'fullscreen_toolbar_probe',
                           '--locked'], cwd=ROOT, env=env)
    checks = {}
    with tempfile.TemporaryDirectory(prefix='zgx-fullscreen-') as stage:
        binary = pathlib.Path(stage) / 'fullscreen_toolbar_probe'
        shutil.copy2(pathlib.Path(args.target_dir) / 'debug/examples/fullscreen_toolbar_probe', binary)
        clean = {key: os.environ[key] for key in ('HOME', 'TMPDIR', 'USER') if key in os.environ}
        clean['PATH'] = '/usr/bin:/bin'
        for via in ('green', 'borderless'):
            out = subprocess.run([str(binary), '--via', via], cwd='/tmp', env=clean, capture_output=True,
                                 text=True, timeout=60).stdout
            print(f'== {via}')
            print('\n'.join(line for line in out.splitlines() if re.match(r'(STRIP|WINDOW|NO_WINDOW) ', line)))
            windows = {m[0]: m for m in re.findall(
                r'^WINDOW (\S+) fullscreen=(\S+) frame_h=(\d+) layout_h=(\d+) buttons_hidden=(\S+) toolbar_visible=(\S+)$',
                out, re.M)}
            strips = re.findall(r'^STRIP full height=(\d+) alpha=([\d.]+) visible=(\S+)$', out, re.M)
            before, full, after = windows.get('before'), windows.get('full'), windows.get('after')
            checks[f'{via}: done'] = 'FULLSCREEN_TOOLBAR_DONE' in out and None not in (before, full, after)
            if None in (before, full, after):
                continue
            checks[f'{via}: before, the inset strip leaves the layout rect short'] = (
                before[1] == 'false' and int(before[3]) < int(before[2]) and before[5] == 'true')
            checks[f'{via}: in full screen'] = full[1] == 'true'
            checks[f'{via}: in full screen, the inset toolbar is hidden'] = full[5] == 'false'
            checks[f'{via}: in full screen, the content layout rect is the whole window'] = full[3] == full[2]
            checks[f'{via}: in full screen, no strip shows over the top'] = all(
                float(alpha) == 0 or visible == 'false' for _, alpha, visible in strips)
            checks[f'{via}: after, out of full screen'] = after[1] == 'false'
            checks[f'{via}: after, the toolbar and traffic lights are back'] = (
                after[5] == 'true' and after[4] == 'false')
            checks[f'{via}: after, the inset strip is back'] = after[3] == before[3] and after[2] == before[2]
    for name, ok in checks.items():
        print(('ok  ' if ok else 'FAIL') + ' ' + name)
    return 0 if checks and all(checks.values()) else 1


if __name__ == '__main__':
    sys.exit(main())
