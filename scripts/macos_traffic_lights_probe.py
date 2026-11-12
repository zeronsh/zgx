#!/usr/bin/env python3
"""An inset title bar's traffic lights where the app puts them, on macOS.

    python3 scripts/macos_traffic_lights_probe.py [--target-dir DIR]

Builds the traffic_lights_probe example and runs it twice, with the close
button asked at 18,22 and at 12,6 (`WindowOptions::traffic_lights`, content
points from the window's top left). Through a resize, the button moved off its place and read back at once (AppKit's
layout moves it by the same frame change), a dark and a light
appearance, the app hidden and shown (focus lost and back), a zoom to 125%
and back, and into and out of full screen (as the green button does), AppKit
must have the close button's top left there, read from the button's own
frame: at 125% where the zoomed content has it (x × 1.25, centred on the
row a button there centres on at 100%). The window must say so too
(`WindowInfo::controls`, unzoomed to the content's points) and its title
bar strip (`titlebar_area`) must be as tall as centres them, or AppKit's
40 points where they sit higher. In full screen it reports no buttons.

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
    subprocess.check_call(['cargo', 'build', '-q', '-p', 'zgx-desktop', '--example', 'traffic_lights_probe',
                           '--locked'], cwd=ROOT, env=env)
    checks = {}
    with tempfile.TemporaryDirectory(prefix='zgx-traffic-lights-') as stage:
        binary = pathlib.Path(stage) / 'traffic_lights_probe'
        shutil.copy2(pathlib.Path(args.target_dir) / 'debug/examples/traffic_lights_probe', binary)
        clean = {key: os.environ[key] for key in ('HOME', 'TMPDIR', 'USER') if key in os.environ}
        clean['PATH'] = '/usr/bin:/bin'
        for x, y in ((18, 22), (12, 6)):
            out = subprocess.run([str(binary), '--at', f'{x},{y}'], cwd='/tmp', env=clean, capture_output=True,
                                 text=True, timeout=60).stdout
            print(f'== at {x},{y}')
            print('\n'.join(line for line in out.splitlines() if line.startswith('CLOSE ')))
            reports = {when: dict(re.findall(r'(\w+)=(\S+)', rest))
                       for when, rest in re.findall(r'^CLOSE (\S+) (.*)$', out, re.M)}
            label = f'at {x},{y}'
            checks[f'{label}: done'] = 'TRAFFIC_LIGHTS_DONE' in out and len(reports) == 10
            for when, report in reports.items():
                if when == 'full':
                    checks[f'{label}: in full screen, no buttons reported'] = report['controls'] == 'none'
                    continue
                zoom, height = float(report['zoom']), float(report['h'])
                want_x, want_y = x * zoom, (y + height / 2) * zoom - height / 2
                checks[f'{label}: {when}, the close button at {want_x:g},{want_y:g}'] = (
                    abs(float(report['x']) - want_x) <= 0.5 and abs(float(report['y']) - want_y) <= 0.5)
                controls = [float(v) for v in report['controls'].split(',')] if report['controls'] != 'none' else []
                checks[f'{label}: {when}, the window says so'] = len(controls) == 4 and (
                    abs(controls[0] - want_x / zoom) <= 0.5 and abs(controls[1] - want_y / zoom) <= 0.5)
                # AppKit's 40-point strip, in the window's points, or taller.
                strip = max(40, want_y * 2 + height) / zoom
                checks[f'{label}: {when}, the strip as tall as centres them'] = (
                    report['area_h'] != 'none' and abs(float(report['area_h']) - strip) <= 0.5)
    for name, ok in checks.items():
        print(('ok  ' if ok else 'FAIL') + ' ' + name)
    return 0 if checks and all(checks.values()) else 1


if __name__ == '__main__':
    sys.exit(main())
