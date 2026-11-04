#!/usr/bin/env python3
"""An inset title bar in full screen on macOS: no opaque strip over the top.

    python3 scripts/macos_fullscreen_toolbar_probe.py [--target-dir DIR]

Builds the fullscreen_toolbar_probe example and runs it four times: into
and out of full screen as the green button does (`toggleFullScreen:`), by
clicking it (`performClick:` on the zoom button), with
zgx's `set_fullscreen` (winit's borderless full screen), and as a transition
that fails after it began (AppKit's will-enter notification with no full
screen after it, and `windowDidFailToEnterFullScreen:` to the window's
delegate, as AppKit tells winit: the toolbar must come back). In full screen,
AppKit's toolbar window above it (NSToolbarFullScreenWindow) must show
nothing (alpha 0, or not visible), the inset toolbar must be hidden and the
content layout rect must be the window's whole height. After, the toolbar
and the traffic lights are back and the inset strip leaves the content
layout rect shorter again.

Each run also checks what the window says as it goes (`STATE` lines:
`WindowInfo::fullscreen`, `WindowContext::full_screen_transition`, and the
traffic lights, `controls`). By the green button: entering, then in full
screen with no transition and no traffic lights, then leaving, then out with
them back; borderless the same (AppKit animates winit's borderless full
screen too). Failed: entering, then no transition once it has had time to
end, still out of full screen, the traffic lights back.

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
        for via in ('green', 'button', 'borderless', 'failed'):
            out = subprocess.run([str(binary), '--via', via], cwd='/tmp', env=clean, capture_output=True,
                                 text=True, timeout=60).stdout
            print(f'== {via}')
            print('\n'.join(line for line in out.splitlines() if re.match(r'(STRIP|WINDOW|NO_WINDOW|STATE) ', line)))
            states = re.findall(r'^STATE \d+ full=(\S+) transition=(\S+) ([\d.]+) controls=(\S+)$', out, re.M)
            seen = [(full == 'true', moving, controls != 'none') for full, moving, _, controls in states]
            def went(*steps):
                # `steps` in order, each a state seen (others between); None
                # matches either.
                at = 0
                for state in seen:
                    if at < len(steps) and all(want is None or want == got for want, got in zip(steps[at], state)):
                        at += 1
                return at == len(steps)
            out_with_lights = (False, 'none', True)
            if via == 'failed':
                checks[f'{via}: entering, then no transition once it had time to end'] = went(
                    out_with_lights, (False, 'entering', None), out_with_lights) and seen[-1] == out_with_lights
            else:
                # The green button, a click on it, or winit's borderless full
                # screen (AppKit animates each): out, entering (before the
                # window says it is in), in with no traffic lights, leaving,
                # out with them back.
                checks[f'{via}: entering, then in full screen, no transition, no traffic lights'] = went(
                    out_with_lights, (None, 'entering', None), (True, 'none', False))
                checks[f'{via}: entering begins before full_screen turns true'] = next(
                    (state[0] for state in seen if state[1] == 'entering'), True) is False
                checks[f'{via}: then leaving, then out, the traffic lights back'] = went(
                    (True, 'none', False), (None, 'leaving', None), out_with_lights) and seen[-1] == out_with_lights
                checks[f'{via}: a transition says how long it takes'] = any(
                    moving != 'none' for _, moving, _, _ in states) and all(
                    0 < float(seconds) < 5 for _, moving, seconds, _ in states if moving != 'none')
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
            if via == 'failed':
                # Will-enter with no full screen after it: hidden at first,
                # back once the window is plainly not in full screen.
                checks[f'{via}: going in, the inset toolbar is hidden'] = full[1] == 'false' and full[5] == 'false'
                checks[f'{via}: not in full screen after all, the toolbar is back'] = (
                    after[1] == 'false' and after[5] == 'true' and after[4] == 'false')
                checks[f'{via}: and the inset strip with it'] = after[3] == before[3] and after[2] == before[2]
                continue
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
