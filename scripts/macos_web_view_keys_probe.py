#!/usr/bin/env python3
"""An app's shortcuts while a web page's text field has the keyboard (macOS),
off-screen: no window is ever shown (ZGX_OFFSCREEN=1).

    python3 scripts/macos_web_view_keys_probe.py [--target-dir DIR] [--no-build]

Builds and runs the web_view_keys_probe example. Its menu items' keys are
reserved as a browser keeps its own (⌘T ⌘W ⌘L ⌘K ⌘1 ⌘/ ⌘, ⌃Tab): each fires
the app's action and the page hears none of them. ⌘U, a menu item not
reserved, is the page's first and then the app's. ⌘C and ⌘A, the Edit
menu's, stay the page's field's: the page hears them and no app action fires.

The keys are sent as AppKit sends a key window's (the window's views, the
main menu, the first responder), straight to the hidden window: the user's
keyboard and other apps hear nothing. macOS only.
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
RESERVED = ['new-tab', 'close-tab', 'address', 'palette', 'tab-1', 'model', 'settings', 'next-tab']


def presses(out):
    """Each PRESS line's name, with the lines after it until the next."""
    found, name = {}, None
    for line in out.splitlines():
        if line.startswith('PRESS '):
            name = line.split(' ', 1)[1]
            found[name] = []
        elif name is not None:
            found[name].append(line)
    return found


def check(out):
    problems = []
    if 'WEB_KEYS_DONE' not in out:
        problems.append('the probe never finished')
    if 'FIRST Some("NSKVONotifying_ZgxWebView")' not in out and 'FIRST Some("ZgxWebView")' not in out:
        problems.append('the page never had the keyboard')
    seen = presses(out)
    for name in RESERVED:
        lines = seen.get(name, [])
        if f'ACTION {name}' not in lines:
            problems.append(f'reserved {name}: the app never had it')
        if any(line.startswith('PAGE ') for line in lines):
            problems.append(f'reserved {name}: the page heard it')
    lines = seen.get('unreserved', [])
    if 'PAGE meta+u' not in lines:
        problems.append('unreserved ⌘U: the page never heard it first')
    if 'ACTION unreserved' not in lines:
        problems.append('unreserved ⌘U: the page left it alone and the app never had it')
    for name, key in [('copy', 'c'), ('select-all', 'a')]:
        lines = seen.get(name, [])
        if f'PAGE meta+{key}' not in lines:
            problems.append(f'{name}: the page\'s field never heard it')
        if any(line.startswith('ACTION ') for line in lines):
            problems.append(f'{name}: an app action fired')
    return problems


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--target-dir', default=os.environ.get('CARGO_TARGET_DIR', str(ROOT / 'target')))
    parser.add_argument('--no-build', action='store_true', help='run the example as already built')
    args = parser.parse_args()
    if platform.system() != 'Darwin':
        print('macOS only')
        return 2
    if not args.no_build:
        env = dict(os.environ, CARGO_TARGET_DIR=args.target_dir)
        subprocess.run(['cargo', 'build', '-q', '-p', 'zgx-desktop', '--example', 'web_view_keys_probe', '--locked'],
                       cwd=ROOT, env=env, check=True)
    with tempfile.TemporaryDirectory(prefix='zgx-web-keys-') as stage:
        binary = pathlib.Path(stage) / 'web_view_keys_probe'
        shutil.copy2(pathlib.Path(args.target_dir) / 'debug/examples/web_view_keys_probe', binary)
        clean = {key: os.environ[key] for key in ('HOME', 'TMPDIR', 'USER') if key in os.environ}
        clean['PATH'] = '/usr/bin:/bin'
        clean['ZGX_OFFSCREEN'] = '1'
        run = subprocess.run([str(binary)], cwd='/tmp', env=clean, capture_output=True, text=True, timeout=90)
    print(run.stdout, end='')
    problems = check(run.stdout)
    if run.returncode != 0:
        problems.append(f'the probe exited {run.returncode}: {run.stderr[-2000:]}')
    for problem in problems:
        print('FAIL', problem)
    if problems:
        return 1
    print('PASS web view keys: reserved shortcuts are the app\'s, the field keeps its editing keys')
    return 0


if __name__ == '__main__':
    sys.exit(main())
