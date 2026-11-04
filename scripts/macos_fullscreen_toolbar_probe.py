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
    parser.add_argument('--frames', action='store_true',
                        help='measure instead: what the window draws through each transition (no checks)')
    parser.add_argument('--covered', action='store_true',
                        help='with --frames, also with ZGX_RENDER_WHEN_COVERED=1 (occlusion ignored)')
    parser.add_argument('--count', type=int,
                        help='launch it this many times and class each from its log; a launch that did not go '
                             'in and out counts as invalid, and invalid launches fail the batch')
    parser.add_argument('--route', default='green', help='with --count: the route (green, button, borderless)')
    parser.add_argument('--close-button', action='store_true', help='with --count: close by the red button')
    parser.add_argument('--logs', type=pathlib.Path, help='with --count: keep each log and sample here')
    parser.add_argument('--live', action='store_true',
                        help='the window animates itself (WindowOptions::live_full_screen)')
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
        if args.count:
            return count(binary, clean, args)
        if args.frames:
            return frames(binary, clean, args.covered, args.live)
        live = ['--live'] if args.live else []
        for via in ('green', 'button', 'borderless', 'failed'):
            out = subprocess.run([str(binary), '--via', via, *live], cwd='/tmp', env=clean, capture_output=True,
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
                # Through it the traffic lights say where they end up: gone as
                # it begins going in, back as it begins coming out, and never
                # anywhere else (AppKit's place partway).
                entering = next(state for state in seen if state[1] == 'entering')
                leaving = next(state for state in seen if state[1] == 'leaving')
                places = {controls for _, _, _, controls in states if controls != 'none'}
                checks[f'{via}: as it begins going in, no traffic lights'] = not entering[2]
                checks[f'{via}: as it begins coming out, the traffic lights where they end up'] = leaving[2]
                checks[f'{via}: the traffic lights only ever in their place'] = len(places) == 1
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
        def animated(via):
            return subprocess.run([str(binary), '--via', via, '--animate', *live], cwd='/tmp', env=clean, capture_output=True,
                                  text=True, timeout=60).stdout

        def frames_in(out, start, end):
            return sum(start <= int(at) <= end for at in re.findall(r'^FRAME (\d+) ', out, re.M))

        def spans(out):
            found, began = [], None
            for at, moving, seconds in re.findall(r'^STATE (\d+) full=\S+ transition=(\S+) ([\d.]+) ', out, re.M):
                if moving != 'none' and began is None:
                    began = (int(at), moving, float(seconds))
                elif moving == 'none' and began:
                    found.append((*began, int(at)))
                    began = None
            return found

        # Drawn through a transition (behind AppKit's snapshot), and its
        # second one each way says how long the first took.
        taken = spans(animated('twice'))
        checks['twice: two transitions each way'] = [span[1] for span in taken] == ['entering', 'leaving'] * 2
        if len(taken) == 4:
            for first, second in zip(taken[:2], taken[2:]):
                real = (first[3] - first[0]) / 1000
                checks[f'twice: {first[1]} the second time says what the first took ({second[2]} s, {real} s)'] = (
                    abs(second[2] - real) < 0.1)
        out = animated('green')
        for start, moving, _, end in spans(out):
            inside = frames_in(out, start, end)
            checks[f'green: drawn through {moving} ({inside} frames in {end - start} ms)'] = inside >= (end - start) / 50
        if args.live:
            # The live window goes there a step at a time, on the screen, and
            # what is drawn is its size at every frame.
            sized = [(int(at), (int(ww), int(wh)), (int(dw), int(dh))) for at, ww, wh, dw, dh in re.findall(
                r'^FRAME (\d+) window=(\d+)x(\d+) drawn=(\d+)x(\d+)$', out, re.M)]
            covered = [int(at) for at in re.findall(r'^OCCLUSION (\d+) visible=false$', out, re.M)]
            for start, moving, _, end in spans(out):
                checks[f'live: {moving}, AppKit never counts the window covered'] = not any(
                    start <= at <= end for at in covered)
                inside = [row for row in sized if start <= row[0] <= end]
                windows = [row[1] for row in inside]
                steps = [abs(b[0] - a[0]) for a, b in zip(windows, windows[1:])]
                checks[f'live: {moving} goes a step at a time ({len(set(windows))} sizes, largest {max(steps, default=0)} pt)'] = (
                    len(set(windows)) >= 20 and max(steps, default=9999) <= 200)
                # A frame's two reads straddle a resize by at most a step.
                behind = max((abs(w[0] - d[0]) for _, w, d in inside), default=0)
                checks[f'live: {moving} draws at the window\'s size (at most {behind} pt behind)'] = behind <= 200
            first, last = sized[0][1:], sized[-1][1:]
            checks[f'live: back at its size ({first[0]} then {last[0]})'] = last[0] == sized[10][1]
        # Genuinely hidden, it stops drawing, and draws again as it comes back.
        for via, wait in (('hidden', 1000), ('hidden-during', 3000)):
            out = animated(via)
            marks = dict((name, int(at)) for at, name in re.findall(r'^MARK (\d+) (\S+)$', out, re.M))
            if set(marks) != {'hidden', 'back'}:
                checks[f'{via}: done'] = False
                continue
            still = frames_in(out, marks['hidden'] + wait, marks['back'])
            again = frames_in(out, marks['back'] + 500, marks['back'] + 1500)
            before = frames_in(out, marks['hidden'] - 500, marks['hidden'])
            print(f'== {via}: {before} frames before, {still} hidden, {again} back')
            checks[f'{via}: drawing before'] = before > 5
            checks[f'{via}: minimized, it stops drawing ({still} frames)'] = still <= 2
            checks[f'{via}: back, it draws again ({again} frames)'] = again > 5
    for name, ok in checks.items():
        print(('ok  ' if ok else 'FAIL') + ' ' + name)
    return 0 if checks and all(checks.values()) else 1


def count(binary, clean, args):
    """`args.count` launches of a route, each classed from its log: `ok` only
    if it went in, was in full screen, came out, and closed by itself."""
    logs = args.logs or pathlib.Path(tempfile.mkdtemp(prefix='zgx-fullscreen-count-'))
    logs.mkdir(parents=True, exist_ok=True)
    flags = ['--via', args.route] + (['--live'] if args.live else []) + (
        ['--close-button'] if args.close_button else [])
    tally = {'ok': 0, 'invalid': 0, 'hang-hover': 0, 'hang-other': 0}
    clean = dict(clean, ZGX_TRACE_ZOOM_HOVER='1')
    ignored = 0
    for i in range(1, args.count + 1):
        log = logs / f'{args.route}-{i}.log'
        with log.open('w') as out:
            proc = subprocess.Popen([str(binary), *flags], cwd='/tmp', env=clean, stdout=out, stderr=subprocess.STDOUT)
            try:
                proc.wait(timeout=25)
                hung = False
            except subprocess.TimeoutExpired:
                hung = True
                sample = logs / f'{args.route}-{i}.sample.txt'
                subprocess.run(['sample', str(proc.pid), '3', '-file', str(sample)], capture_output=True)
                proc.terminate()
                proc.wait()
        text = log.read_text()
        ignored += 'ZOOM_HOVER ignored' in text
        went = all(re.search(pattern, text, re.M) for pattern in (
            r'^STATE \d+ full=\S+ transition=entering ',
            r'^STATE \d+ full=true transition=none ',
            r'^STATE \d+ full=\S+ transition=leaving ',
            r'^STATE \d+ full=false transition=none ',
            r'^WINDOW full fullscreen=true ',
            r'^FULLSCREEN_TOOLBAR_DONE$',
        )) and 'NO_WINDOW' not in text
        if hung:
            stack = sample.read_text(errors='replace') if sample.exists() else ''
            kind = 'hang-hover' if '_hoverTriggeredShowMenuForView' in stack else 'hang-other'
            tally[kind if went else 'invalid'] += 1
            print(f'{i}: {kind}{"" if went else " (never went in and out: invalid)"}, {log}', flush=True)
        elif went and proc.returncode == 0:
            tally['ok'] += 1
        else:
            tally['invalid'] += 1
            print(f'{i}: invalid, exit {proc.returncode}, {log}', flush=True)
    print(f'{" ".join(flags)}: {args.count} launches: ' + ', '.join(f'{v} {k}' for k, v in tally.items())
          + f'; AppKit\'s hover menu stopped in {ignored}')
    return 0 if tally['ok'] == args.count else 1


def frames(binary, clean, covered, live):
    """Each route with `--animate`: for each transition (`STATE` lines from
    its start to its end), the frames drawn inside it, the longest gap
    between frames around it, and AppKit's occlusion changes then."""
    runs = [('', clean, [])] + ([('covered', dict(clean, ZGX_RENDER_WHEN_COVERED='1'), [])] if covered else []) + (
        [('live', clean, ['--live'])] if live else [])
    for label, env, flags in runs:
        for via in ('green', 'borderless'):
            out = subprocess.run([str(binary), '--via', via, '--animate', *flags], cwd='/tmp', env=env,
                                 capture_output=True, text=True, timeout=60).stdout
            sized = [(int(at), (int(ww), int(wh)), (int(dw), int(dh))) for at, ww, wh, dw, dh in re.findall(
                r'^FRAME (\d+) window=(\d+)x(\d+) drawn=(\d+)x(\d+)$', out, re.M)]
            frames_at = [at for at, _, _ in sized]
            occlusion = re.findall(r'^OCCLUSION (\d+) visible=(\S+)$', out, re.M)
            states = re.findall(r'^STATE (\d+) full=(\S+) transition=(\S+) ([\d.]+) controls=(\S+)$', out, re.M)
            spans, began = [], None
            for at, full, moving, seconds, controls in states:
                if moving != 'none' and began is None:
                    began = (int(at), moving, float(seconds))
                elif moving == 'none' and began:
                    spans.append((*began, int(at)))
                    began = None
            print(f'== {via}{" " + label if label else ""}: {len(frames_at)} frames')
            for start, moving, seconds, end in spans:
                inside = [t for t in frames_at if start <= t <= end]
                around = [t for t in frames_at if start - 200 <= t <= end + 200]
                gaps = [b - a for a, b in zip(around, around[1:])]
                seen = [f'{at}:{visible}' for at, visible in occlusion if start - 200 <= int(at) <= end + 200]
                print(f'{moving} {start}..{end} ms ({end - start} ms, AppKit says {seconds * 1000:.0f}): '
                      f'{len(inside)} frames inside, longest gap {max(gaps, default=0)} ms, '
                      f'occlusion {" ".join(seen) or "none"}')
                inside = [row for row in sized if start <= row[0] <= end]
                windows = [row[1] for row in inside]
                drawn = [row[2] for row in inside]
                steps = [abs(b[0] - a[0]) for a, b in zip(windows, windows[1:])]
                print(f'  window sizes {len(set(windows))}, drawn sizes {len(set(drawn))}, '
                      f'largest window step {max(steps, default=0)} pt, '
                      f'drawn behind window by at most {max((abs(w[0] - d[0]) for w, d in zip(windows, drawn)), default=0)} pt, '
                      f'{windows[0] if windows else None} .. {windows[-1] if windows else None}')
            print('controls: ' + ' '.join(f'{at}:{controls}' for at, _, _, _, controls in states))
    return 0


if __name__ == '__main__':
    sys.exit(main())
