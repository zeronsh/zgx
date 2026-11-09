#!/usr/bin/env python3
"""Permanent scroll gate. See docs/scroll-benchmark.md for the measurement contract."""
import argparse
from concurrent.futures import ThreadPoolExecutor
from collections import Counter
import hashlib
import itertools
import json
import math
import os
from pathlib import Path
import platform
import re
import shutil
import socket
import subprocess
import sys
import tempfile
import time

ROOT = Path(__file__).resolve().parents[2]
SIZES = {'S': (640, 480), 'M': (960, 720), 'L': (1440, 960)}
SCENES = ('transcript', 'tables', 'plain', 'grid', 'nested')
GESTURES = ('wing', 'slow', 'flick', 'reversals', 'notches', 'fling', 'stream')


def percentile(values, q):
    if not values:
        return None
    return sorted(values)[max(0, math.ceil(len(values) * q) - 1)]


def spread(values):
    return {key: percentile(values, q) for key, q in [('p50', .5), ('p99', .99), ('max', 1)]}


def tape(gesture, hz):
    if gesture == 'wing':
        return (ROOT / 'scripts/perf/scroll-tapes/wing.tape').read_text()
    if gesture == 'reference':
        return '# zgx wheel tape 1\n0 pixel 0 0 started\n10000 pixel 0 0 ended\n'
    events = [(0., 'pixel', 0., 0., 'started')]
    period = 1000 / hz
    for i in range(1, hz * 4 + 1):
        t = i * period
        dx = 0
        phase = 'moved'
        unit = 'pixel'
        if gesture == 'slow':
            dy = -.2  # also tests quantized, intentionally unchanged pixels
        elif gesture in ('flick', 'fling'):
            dy = -(64 if gesture == 'fling' else 32) * math.exp(-i / (hz * 1.4))
        elif gesture == 'reversals':
            dy = -12 if (i // (hz // 4)) % 2 == 0 else 12
        elif gesture == 'notches':
            if i % (hz // 8):
                continue
            unit, dy, phase = 'lines', -3, 'moved'
        else:
            dy = -8
        events.append((t, unit, dx, dy, phase))
    events.append((4010., 'pixel', 0., 0., 'ended'))
    return '# zgx wheel tape 1\n' + ''.join(f'{t:.3f} {u} {x:.6f} {y:.6f} {p}\n' for t, u, x, y, p in events)


def parse_tape(text):
    out = []
    for line in text.splitlines():
        if not line or line.startswith('#'):
            continue
        t, unit, x, y, phase = line.split()
        out.append((float(t), unit, float(x), float(y), phase))
    return out


def motion_windows(events, period, scale):
    """Only slots with enough input for an actual device-pixel move demand a frame.

    Idle tape gaps and subpixel accumulation are not missed refreshes. Fill
    between dense moving events, never across an ended gesture or sparse
    mouse notches. Uses absolute displacement so reversals cannot cancel.
    """
    windows = []
    pending = 0.
    last = None
    for t, unit, dx, dy, phase in events:
        if phase in ('ended', 'cancelled'):
            last = None
            pending = 0.
            continue
        distance = (abs(dx) + abs(dy)) * (40 if unit == 'lines' else 1) * scale
        if distance == 0:
            continue
        pending += distance
        if pending >= 1:
            start = t
            windows.append((start, t + period))
            pending %= 1
            last = t
    merged = []
    for start, end in windows:
        if merged and start <= merged[-1][1] + .02:
            merged[-1] = (merged[-1][0], max(end, merged[-1][1]))
        else:
            merged.append((start, end))
    return merged


def active_gap(a, b, windows, period):
    if b - a <= 1.5 * period:
        return any(start <= b and end >= a for start, end in windows)
    # Dense motion must have demanded an intermediate refresh. Two isolated
    # pixel changes several refreshes apart are an intentionally idle gap.
    return any(end - start >= 1.5 * period and
               start <= (a + b) / 2 <= end for start, end in windows)


def app_work_ms(frame):
    # Present includes native drawable/compositor waits, and GPU work has its
    # own timestamp. Keep these out of the app's main-thread work budget.
    return sum(p['micros'] for p in frame['phases'] if p['phase'] not in ('wait', 'present')) / 1000


def summarize(frames, presents, events, hz, scale, anchor, rss, *, reference=False,
              main_budget=4., gpu_budget=4.):
    period = 1000 / hz
    windows = [(0, events[-1][0])] if reference and events else motion_windows(events, period, scale)
    # Feedback carries the exact trace index captured before presenting. An
    # older runner's two-field record remains readable but cannot certify
    # attribution: missing frame identity is an explicit gate failure.
    records = [(r[0], r[1], int(r[2]) if len(r) > 2 else 0,
                r[3] if len(r) > 3 else 0) for r in presents]
    shown = sorted((r for r in records if r[1] > 0 and r[1]*1000 >= anchor), key=lambda r:r[1])
    gaps, idle, late_records = [], [], []
    by_id = {f['frame']: f for f in frames}

    def attribute(kind, at, ids):
        candidates = [by_id[i] for i in ids if i in by_id]
        main = [app_work_ms(f) for f in candidates]
        gpu = [f['gpu_micros']/1000 for f in candidates if f.get('gpu_micros') is not None]
        notes = [n for f in candidates for n in f.get('notes', [])]
        if any(v > main_budget for v in main):
            cause = 'app'
        elif any(v > gpu_budget for v in gpu):
            cause = 'gpu'
        elif any("waited a refresh more to keep" in n or "waiting for an earlier frame" in n for n in notes):
            cause = 'app_pacing'
        elif candidates and len(gpu) == len(candidates):
            cause = 'system'
        else:
            cause = 'unattributed'
        return dict(kind=kind, presented_ms=at, cause=cause,
                    frames=[f['frame'] for f in candidates], main_max_ms=max(main, default=None),
                    gpu_max_ms=max(gpu, default=None), notes=notes)

    for prev, current in zip(shown, shown[1:]):
        a, b = prev[1]*1000-anchor, current[1]*1000-anchor
        if not active_gap(a, b, windows, period):
            idle.append(b-a)
            continue
        gaps.append(b-a)
        if b-a > 1.5*period:
            ids = range(prev[2], current[2]+1) if prev[2] and current[2] >= prev[2] else []
            late_records.append(attribute('late', b, ids))
    skipped = [r for r in records if r[1] <= 0 and r[0]*1000 >= anchor]
    for r in skipped:
        late_records.append(attribute('skipped', r[0]*1000-anchor, [r[2]] if r[2] else []))
    copied = repainted = pictures = rows = 0
    blockers = Counter()
    gpu, instructions, work, scroll_frames = [], [], [], []
    for f in frames:
        notes = f.get('notes', [])
        moving = any('copied its pixels' in n or 'repainted its view:' in n or
                     'composited its picture' in n for n in notes)
        if not moving and not reference:
            continue
        scroll_frames.append(f)
        work.append(app_work_ms(f))
        instructions.append(sum(p['instructions'] for p in f['phases'] if p['phase'] != 'wait'))
        if f.get('gpu_micros') is not None:
            gpu.append(f['gpu_micros']/1000)
        rows += notes.count('list built row')
        copied += any('copied its pixels' in n for n in notes)
        pictures += any('composited its picture' in n for n in notes)
        repainted += any('repainted its view:' in n for n in notes)
        for n in notes:
            if 'repainted its view:' in n:
                blockers[n.split('repainted its view:', 1)[1].strip()] += 1
    causes = Counter(r['cause'] for r in late_records)
    app_late = sum(causes[c] for c in ('app', 'gpu', 'app_pacing'))
    total = len(gaps)+len(skipped)
    return dict(frames=len(frames), scroll_frames=len(scroll_frames), active_intervals=len(gaps),
                late_frames=sum(r['kind']=='late' for r in late_records), skipped_drawables=len(skipped),
                late_pct=100*len(late_records)/total if total else None,
                app_late_frames=app_late, app_late_pct=100*app_late/total if total else None,
                system_late_frames=causes['system'], unattributed_late_frames=causes['unattributed'],
                late_attributions=late_records, interval_ms=spread(gaps), idle_intervals=len(idle),
                idle_interval_ms=spread(idle), main_over_4ms=sum(v>4 for v in work),
                main_work_ms=spread(work), gpu_p99_ms=percentile(gpu, .99),
                instructions=spread(instructions), copied_frames=copied, repainted_frames=repainted,
                picture_frames=pictures, copy_pct=100*copied/max(1,len(scroll_frames)),
                blockers=dict(blockers), rows_built_in_scroll_frames=rows, rss_kib=rss)


def failures(metrics, args):
    errors = []
    if metrics['active_intervals'] < args.min_intervals:
        errors.append('insufficient actual presentation feedback during motion')
    if metrics['scroll_frames'] < 10:
        errors.append('no sustained scrolling (wrong target, clamped edge, or missing trace)')
    if metrics['app_late_pct'] is not None and metrics['app_late_pct'] > args.max_late:
        errors.append(f"app/GPU-attributed late frames: {metrics['app_late_frames']} (target zero)")
    if metrics['unattributed_late_frames']:
        errors.append(f"late frames missing attribution: {metrics['unattributed_late_frames']}")
    for key in ('p99', 'max'):
        value = metrics['main_work_ms'][key]
        if value is None or value > args.max_main_ms:
            errors.append(f"main-thread work {key} {value} ms exceeds {args.max_main_ms} ms")
    if metrics['rows_built_in_scroll_frames'] > args.max_rows:
        errors.append(f"rows built in scroll frames: {metrics['rows_built_in_scroll_frames']} > {args.max_rows}")
    if metrics['repainted_frames']:
        errors.append(f"full scroll-view repaints: {metrics['repainted_frames']} (target zero)")
    if metrics['instructions']['max'] is None or metrics['instructions']['max'] == 0:
        errors.append('main-thread instruction counter unavailable')
    elif metrics['instructions']['max'] > args.max_instructions:
        errors.append('scroll-frame instructions exceed the absolute ceiling')
    if metrics['gpu_p99_ms'] is None:
        errors.append('GPU timings unavailable')
    elif metrics['gpu_p99_ms'] >= args.max_gpu_ms:
        errors.append('GPU p99 over budget')
    return errors


def request(pid, cmd, timeout=10, **kw):
    directory = Path(tempfile.gettempdir()) / f'zgx-inspect-{os.getuid()}'
    with socket.socket(socket.AF_UNIX) as s:
        s.settimeout(timeout)
        s.connect(str(directory / f'{pid}.sock'))
        s.sendall((json.dumps(dict(cmd=cmd, **kw)) + '\n').encode())
        with s.makefile('r') as reader:
            result = json.loads(reader.readline())
    if not result.get('ok'):
        raise RuntimeError(result.get('error', result))
    return result


def rss(pid):
    return int(subprocess.check_output(['ps', '-o', 'rss=', '-p', str(pid)], text=True).strip())


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def collect(pid, future, begin, out):
    cursor = begin['frame']
    frames, polls = {}, []
    while True:
        queried = time.monotonic()
        batch = request(pid, 'frames', n=400, since=cursor, runs=0)['data']
        with (out / 'frames.jsonl').open('a') as raw:
            raw.write(json.dumps(dict(query_ms=(time.monotonic()-queried)*1000, frames=batch)) + '\n')
        if len(batch) >= 400:
            raise RuntimeError('frame ring may have overflowed; refusing a partial sample')
        for f in batch:
            frames[f['frame']] = f
        if batch:
            cursor = max(f['frame'] for f in batch)
        # Numeric cursor is the last frame actually read, so a frame drawn
        # between this read and the next mark is never lost.
        polls.append(request(pid, 'mark', name='drain')['data'])
        if future.done():
            future.result()
            break
        time.sleep(1)
    return sorted(frames.values(), key=lambda f: f['frame']), polls


def run_cell(args, scene, gesture, size, hz, scale, background, repeat, present):
    label = f'{scene}-{gesture}-{size}-{hz}hz-{scale}x-{background}-p{present}-{repeat}'
    out = args.output / label
    out.mkdir(parents=True, exist_ok=False)
    result = dict(scene=scene, gesture=gesture, size=size, hz=hz, scale=scale,
                  background=background, repeat=repeat, present=present, status='invalid', failures=[])
    process = None
    try:
        with tempfile.TemporaryDirectory(prefix='zgx-scroll-') as tmp:
            stage = Path(tmp)
            source = ROOT / 'crates/zgx-script/examples' / ('transcript_scroll' if scene in ('transcript', 'tables') else 'scroll_scenes')
            shutil.copytree(source, stage / 'app')
            width, height = SIZES[size]
            config = {'title': f'zgx scroll gate {label}', 'width': width * scale / args.native_scale, 'height': height * scale / args.native_scale,
                      'min_width': 1, 'min_height': 1, 'titlebar': 'hidden', 'app_id': 'zgx-scroll-bench'}
            (stage / 'app/zgx.json').write_text(json.dumps(config))
            text = tape(gesture, hz)
            target = '#transcript'
            if scene == 'nested':
                target = '#horizontal'
                text = '# zgx wheel tape 1\n' + ''.join(
                    f'{t:.3f} {unit} {dy:.6f} 0 {phase}\n'
                    for t, unit, dx, dy, phase in parse_tape(text))
            (out / 'gesture.tape').write_text(text)
            events = parse_tape(text)
            result['tape_sha256'] = hashlib.sha256(text.encode()).hexdigest()
            env = {k: v for k, v in os.environ.items() if not k.startswith(('ZGX_', 'ZGUI_'))}
            env.update(ZGX_SCROLL_BENCH='1', ZGX_PRESENT_DRAWABLE=present)
            if args.damage_check:
                env['ZGX_DAMAGE_CHECK'] = '1'
                env['ZGX_DAMAGE_DUMP'] = str(out.resolve())
            with (out / 'native.log').open('w') as log:
                process = subprocess.Popen([str(args.runner), '--inspect', str(stage / 'app'), scene, str(hz)],
                                           env=env, stdout=log, stderr=log)
                deadline = time.monotonic() + 30
                while True:
                    if process.poll() is not None:
                        raise RuntimeError(f'window exited {process.returncode} before ready')
                    try:
                        request(process.pid, 'tree')
                        break
                    except (OSError, RuntimeError):
                        if time.monotonic() > deadline:
                            raise RuntimeError('window did not serve inspect within 30s')
                        time.sleep(.2)
                request(process.pid, 'zoom', to=str(scale / args.native_scale))
                time.sleep(args.warmup)
                errors = request(process.pid, 'errors')['text']
                if 'error:' in errors or 'types:' in errors:
                    raise RuntimeError(errors)
                request(process.pid, 'set', name='translucent', value=background == 'translucent')
                reset_offset = 90000
                if gesture == 'stream':
                    request(process.pid, 'set', name='offset', value=1e9)
                    time.sleep(.5)
                    scroll = request(process.pid, 'node', target='#transcript')['data']['scroll']
                    limit = float(re.search(r'of 0 to ([0-9.]+)', scroll).group(1))
                    reset_offset = max(0, limit - 4000)
                request(process.pid, 'set', name='offset', value=reset_offset)
                time.sleep(.5)
                # Warm the exact path once before measuring; cold-entry work
                # has its own cold run option and cannot contaminate this gate.
                warm = tape('fling', hz) if scene != 'idle' else '# zgx wheel tape 1\n0 pixel 0 0 started\n100 pixel 0 0 ended\n'
                request(process.pid, 'replay', target='#transcript', tape=warm, timeout=15)
                request(process.pid, 'set', name='offset', value=reset_offset)
                if scene == 'nested':
                    request(process.pid, 'set', name='horizontal_offset', value=1000)
                time.sleep(.5)
                if gesture == 'stream':
                    if scene != 'transcript':
                        raise RuntimeError('stream gesture requires transcript scene')
                    request(process.pid, 'set', name='stream_row', value=400)
                begin = request(process.pid, 'mark', name='sample')['data']
                result['conditions'] = begin
                if not {'refresh_ms', 'pixel_scale', 'host_ns'} <= begin.keys():
                    raise RuntimeError('runner lacks scroll telemetry metadata; rebuild it')
                for key, expected in [('width', width), ('height', height), ('pixel_scale', scale)]:
                    if abs(begin[key] - expected) > .02:
                        raise RuntimeError(f"unsupported condition: {key}={begin[key]}, requested {expected}")
                if abs(begin['refresh_ms'] - 1000 / hz) > .5:
                    raise RuntimeError(f"unsupported display refresh: {begin['refresh_ms']:.3f} ms, requested {hz} Hz")
                before = rss(process.pid)
                with ThreadPoolExecutor(max_workers=1) as pool:
                    future = pool.submit(request, process.pid, 'replay', target=target, tape=text,
                                         timeout=events[-1][0] / 1000 + 20)
                    frames, polls = collect(process.pid, future, begin, out)
                    result['replay'] = future.result()
                after = rss(process.pid)
                result['polls'] = polls
                result['frames'] = frames
                # Give asynchronous presentation callbacks time to arrive.
                time.sleep(.3)
                presents = []
                replay_starts = []
                latency = []
                log.flush()
                for line in (out / 'native.log').read_text().splitlines():
                    if line.startswith('SCROLL_LATENCY '):
                        latency.append(tuple(map(float, line.split()[1:])))
                    if line.startswith('SCROLL_REPLAY '):
                        replay_starts.append(int(line.split()[1]) / 1e6)
                    match = re.fullmatch(r'SCROLL_PRESENT ([0-9.]+) ([0-9.]+) ([0-9]+) ([0-9.]+)', line)
                    if match:
                        record = tuple(map(float, match.groups()))
                        presents.append(record)
                        if record[3] > 0 and record[1] > 0:
                            latency.append((record[3], record[1]))
                result['presents'] = presents
                if not replay_starts:
                    raise RuntimeError('missing replay clock anchor')
                result['replay_start_host_ms'] = replay_starts[-1]
                metrics = summarize(frames, presents, events, hz, scale, replay_starts[-1],
                                    dict(before=before, after=after, delta=after-before),
                                    reference=scene == 'idle', main_budget=args.max_main_ms, gpu_budget=args.max_gpu_ms)
                metrics['start_to_present_ms'] = spread([(p-s)*1000 for s,p in latency if s*1000 >= replay_starts[-1]])
                result['latency_samples'] = latency
                result['metrics'] = metrics
                result['failures'] = failures(metrics, args)
                latency_p99 = metrics['start_to_present_ms']['p99']
                if present == '1' and latency_p99 is None:
                    result['failures'].append('missing frame start-to-presentation latency')
                if latency_p99 is not None and latency_p99 > args.max_present_latency_ms:
                    result['failures'].append('frame start-to-presentation latency over budget')
                if args.damage_check and 'ZGX_DAMAGE_CHECK frame' in (out / 'native.log').read_text():
                    result['failures'].append('stale pixels detected')
                result['status'] = 'fail' if result['failures'] else 'pass'
    except Exception as error:
        result['failures'].append(str(error))
    finally:
        if process is not None and process.poll() is None:
            process.terminate()
            try:
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait()
        (out / 'run.json').write_text(json.dumps(result, indent=2) + '\n')
    result['evidence'] = str(out)
    return {k:v for k,v in result.items() if k not in ('frames', 'presents', 'latency_samples', 'polls')}


def report(results):
    lines = ['| Scene | Gesture | Size | Hz / scale / bg / present | Late % | Interval p99 ms | Main >4ms | App/GPU / system / unknown late | Result |',
             '|---|---|---|---|---:|---:|---:|---|---|']
    for r in results:
        m = r.get('metrics', {})
        def fmt(v):
            return '—' if v is None else f'{v:.2f}'
        lines.append(f"| {r['scene']} | {r['gesture']} | {r['size']} | {r['hz']} / {r['scale']} / {r['background']} / {r.get('present', '?')} | "
                     f"{fmt(m.get('late_pct'))} | {fmt(m.get('interval_ms', {}).get('p99'))} | {m.get('main_over_4ms', '—')} | {m.get('app_late_frames', '—')} / {m.get('system_late_frames', '—')} / {m.get('unattributed_late_frames', '—')} | {r['status']} |")
    return '\n'.join(lines) + '\n'


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('command', choices=['run', 'headless'])
    p.add_argument('--runner', type=Path, default=ROOT / 'target/dist/zgx-run')
    p.add_argument('--output', type=Path, default=ROOT / 'target/scroll-results' / time.strftime('%Y%m%d-%H%M%S'))
    p.add_argument('--scenes', default='transcript,plain,grid,nested')
    p.add_argument('--gestures', default='slow,flick,reversals,notches,fling,wing,stream')
    p.add_argument('--sizes', default='S,M,L')
    p.add_argument('--hz', default='120')
    p.add_argument('--scale', default='2')
    p.add_argument('--native-scale', type=float, default=2, help='display backing scale; requested render scales use app zoom')
    p.add_argument('--backgrounds', default='opaque,translucent')
    p.add_argument('--present', default='0,1', help='comma-separated 0 (tiles), 1 (drawables), auto')
    p.add_argument('--repeats', type=int, default=3)
    p.add_argument('--warmup', type=float, default=3)
    p.add_argument('--max-late', type=float, default=0.)
    p.add_argument('--max-rows', type=int, default=0)
    p.add_argument('--max-instructions', type=int, default=50_000_000)
    p.add_argument('--max-main-ms', type=float, default=4)
    p.add_argument('--max-gpu-ms', type=float, default=4)
    p.add_argument('--max-present-latency-ms', type=float, default=25)
    p.add_argument('--min-intervals', type=int, default=30)
    p.add_argument('--damage-check', action='store_true', help='correctness run; timings include full repaint verification')
    p.add_argument('--strict', action='store_true', help='headless: require zero row builds and copies on the shared scene')
    args = p.parse_args()
    if args.command == 'headless':
        env = dict(os.environ)
        if args.strict:
            env['ZGX_SCROLL_STRICT'] = '1'
        for scene in args.scenes.split(','):
            env['SCENE'] = scene
            subprocess.run(['cargo', 'run', '--release', '--locked', '-p', 'zgx-script', '--example', 'scroll_bench', '--features', 'inspect'],
                           cwd=ROOT, env=env, check=True)
        return 0
    if sys.platform != 'darwin':
        p.error('native replay currently requires macOS; headless works in normal CI')
    args.runner = args.runner.resolve()
    args.output.mkdir(parents=True, exist_ok=False)
    audit = dict(commit=subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip(),
                 diff=subprocess.check_output(['git', 'diff', 'HEAD'], cwd=ROOT, text=True),
                 runner=str(args.runner), runner_sha256=sha(args.runner), platform=platform.platform(),
                 machine=platform.machine(), args={k: str(v) if isinstance(v, Path) else v for k,v in vars(args).items()})
    sources = args.output / 'sources'
    sources.mkdir()
    for name in ('transcript_scroll', 'scroll_scenes'):
        shutil.copytree(ROOT / 'crates/zgx-script/examples' / name, sources / name)
    shutil.copy2(Path(__file__), sources / 'scroll.py')
    audit['sources'] = {str(f.relative_to(sources)): sha(f) for f in sources.rglob('*') if f.is_file()}
    audit['rustc'] = subprocess.check_output(['rustc', '-vV'], text=True)
    audit['displays'] = subprocess.check_output(['system_profiler', 'SPDisplaysDataType'], text=True)
    (args.output / 'audit.json').write_text(json.dumps(audit, indent=2) + '\n')
    results = []
    references = {}
    cells = itertools.product(range(args.repeats), args.scenes.split(','), args.gestures.split(','),
                              args.sizes.split(','), map(int, args.hz.split(',')), map(float, args.scale.split(',')),
                              args.backgrounds.split(','), args.present.split(','))
    for repeat, scene, gesture, size, hz, scale, bg, present in cells:
        if scene not in SCENES or gesture not in GESTURES or size not in SIZES or present not in ('0', '1', 'auto'):
            p.error('unknown scene, gesture, or size')
        if gesture == 'stream' and scene != 'transcript':
            continue
        print(f'{scene} {gesture} {size} {hz}Hz {scale}x {bg} trial {repeat+1}', flush=True)
        key = (size, hz, scale, bg, present, repeat)
        if key not in references:
            ref = run_cell(args, 'idle', 'reference', size, hz, scale, bg, repeat, present)
            references[key] = ref
            results.append(ref)
        r = run_cell(args, scene, gesture, size, hz, scale, bg, repeat, present)
        r['reference'] = references[key].get('metrics')
        if references[key]['status'] != 'pass':
            r['failures'].append('idle reference failed: system floor cannot be certified')
            r['status'] = 'fail'
        results.append(r)
        print(r['status'], '; '.join(r['failures']), flush=True)
        (args.output / 'summary.md').write_text(report(results))
        (args.output / 'summary.json').write_text(json.dumps(results, indent=2) + '\n')
    print(report(results))
    return int(any(r['status'] != 'pass' for r in results))


if __name__ == '__main__':
    sys.exit(main())
