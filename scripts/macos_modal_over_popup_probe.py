#!/usr/bin/env python3
"""A modal opening over a popup window on macOS: the popup closes.

    python3 scripts/macos_modal_over_popup_probe.py [--target-dir DIR]

Builds the modal_over_popup_probe example and runs it. A dropdown taller
than its window shows in a popup window of its own (above everything in the
window); a modal then opens. The dropdown must close and its popup window
leave the screen, rather than stay drawn over the modal.

It is launched as a direct child, from /tmp, with a clean environment; it
sends no input. If a system dialog appears, stop and report it; never click
it. macOS only.
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
    subprocess.check_call(['cargo', 'build', '-q', '-p', 'zgx-desktop', '--example', 'modal_over_popup_probe',
                           '--locked'], cwd=ROOT, env=env)
    with tempfile.TemporaryDirectory(prefix='zgx-modal-') as stage:
        binary = pathlib.Path(stage) / 'modal_over_popup_probe'
        shutil.copy2(pathlib.Path(args.target_dir) / 'debug/examples/modal_over_popup_probe', binary)
        clean = {key: os.environ[key] for key in ('HOME', 'TMPDIR', 'USER') if key in os.environ}
        clean['PATH'] = '/usr/bin:/bin'
        out = subprocess.run([str(binary)], cwd='/tmp', env=clean, capture_output=True, text=True,
                             timeout=60).stdout
    print('\n'.join(line for line in out.splitlines() if line.startswith('POPUPS ')))
    seen = {m[0]: m for m in re.findall(r'^POPUPS (\S+) windows=(\d+) dropdown=(\S+) modal=(\S+)$', out, re.M)}
    before, after = seen.get('dropdown'), seen.get('modal')
    checks = {
        'done': 'MODAL_OVER_POPUP_DONE' in out and None not in (before, after),
    }
    if None not in (before, after):
        checks.update({
            'the dropdown shows in a popup window': before[2] == 'true' and int(before[1]) >= 1,
            'the modal opened': after[3] == 'true',
            'the dropdown closed as the modal opened': after[2] == 'false',
            'no popup window is left over the modal': int(after[1]) == 0,
        })
    for name, ok in checks.items():
        print(('ok  ' if ok else 'FAIL') + ' ' + name)
    return 0 if all(checks.values()) else 1


if __name__ == '__main__':
    sys.exit(main())
