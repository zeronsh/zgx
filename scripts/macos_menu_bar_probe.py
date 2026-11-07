#!/usr/bin/env python3
"""AppKit's menu bar made once, not on every mouse move, on macOS: with the
bar set to hide itself, an app that activated before its first main menu was
in had AppKit make the bar's windows again for each move (0.3 ms of the app's
CPU a move). Each launch posts 200 moves to its own window and counts what
AppKit made for the bar over them.

    python3 scripts/macos_menu_bar_probe.py [--target-dir DIR]

Builds the menu_bar_probe example and launches it until two launches have
activated before their bar was set (the order the bug needs; most launches
do), at most eight: each must make no more than a few of the bar's views over
its moves. The setting is the launched process's own (`-_HIHideMenuBar YES`):
the user's is untouched, and so is everything outside the process.

Launched as a direct child, from /tmp, with a clean environment. If a system
dialog appears, stop and report it; never click it. macOS only.
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
# A bar made once makes a few views per display; made on every move, about
# two hundred per display.
MOST = 20


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--target-dir', default=os.environ.get('CARGO_TARGET_DIR', str(ROOT / 'target')))
    args = parser.parse_args()
    if platform.system() != 'Darwin':
        print('macOS only')
        return 2
    env = dict(os.environ, CARGO_TARGET_DIR=args.target_dir)
    subprocess.run(['cargo', 'build', '-q', '-p', 'zgx-desktop', '--example', 'menu_bar_probe', '--locked'],
                   cwd=ROOT, env=env, check=True)
    launches = []
    with tempfile.TemporaryDirectory(prefix='zgx-menu-bar-') as stage:
        binary = pathlib.Path(stage) / 'menu_bar_probe'
        shutil.copy2(pathlib.Path(args.target_dir) / 'debug/examples/menu_bar_probe', binary)
        clean = {key: os.environ[key] for key in ('HOME', 'TMPDIR') if key in os.environ}
        clean['PATH'] = '/usr/bin:/bin'
        while len(launches) < 8 and sum(l['order'] == 'activated_first' for l in launches) < 2:
            out = subprocess.run([str(binary), '-_HIHideMenuBar', 'YES'], cwd='/tmp', env=clean,
                                 capture_output=True, text=True, timeout=60).stdout
            print(out, end='')
            found = re.search(r'^MENU_BAR order=(\S+) built_at_launch=(\d+) built_over_200_moves=(\d+)$', out, re.M)
            launches.append({'order': found[1], 'moves': int(found[3])} if found else {'order': 'none', 'moves': -1})
    raced = [l for l in launches if l['order'] == 'activated_first']
    checks = [
        ('every launch reported', all(l['order'] != 'none' for l in launches)),
        ('two launches activated before their bar was set', len(raced) >= 2),
        (f'no launch made over {MOST} of the bar\'s views over its moves',
         all(0 <= l['moves'] <= MOST for l in launches)),
    ]
    failed = [name for name, ok in checks if not ok]
    for name, ok in checks:
        print(f"{'ok  ' if ok else 'FAIL'} {name}")
    if failed:
        print(f'menu_bar_probe: {len(failed)} of {len(checks)} failed', file=sys.stderr)
        return 1
    print(f'menu_bar_probe: {len(checks)} of {len(checks)} passed')
    return 0


if __name__ == '__main__':
    sys.exit(main())
