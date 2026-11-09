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
SIZES = {'S': (640, 480), 'M': (960, 720), 'L': (1440, 960), 'W': (1114, 685)}
SCENES = ('zeron', 'transcript', 'tables', 'plain', 'grid', 'nested')
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
        return '# zgx wheel tape 1\n0 pixel 0 0 moved\n10000 pixel 0 0 moved\n'
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


def motion_windows(events, period, scale, merge=True):
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
    if not merge:
        return windows
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
              main_budget=4., gpu_budget=4., sparse=False):
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
        gpu = [f['gpu_micros']/1000 if f.get('gpu_micros') is not None else 0
               for f in candidates if f.get('gpu_micros') is not None or f.get('spare', False)]
        notes = [n for f in candidates for n in f.get('notes', [])]
        if any(v > main_budget for v in main):
            cause = 'app'
        elif any(v > gpu_budget for v in gpu):
            cause = 'gpu'
        elif any("waited a refresh more to keep" in n or "waiting for an earlier frame" in n for n in notes):
            cause = 'app_pacing'
        else:
            cause = 'unattributed'
        phases = Counter()
        for f in candidates:
            for phase in f['phases']:
                phases[phase['phase']] += phase['micros']/1000
        transitions = [(t, phase) for t, _, _, _, phase in events if phase != 'moved']
        nearby = [dict(at_ms=t, phase=phase, delta_ms=at-t) for t, phase in transitions if abs(at-t) <= 50]
        return dict(kind=kind, presented_ms=at, cause=cause, phase_ms=dict(phases),
                    nearby_phase_changes=nearby,
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
    with_present, present_work, wait_work = [], [], []
    for f in frames:
        notes = f.get('notes', [])
        moving = any('copied its pixels' in n or 'repainted its view:' in n or
                     'composited its picture' in n for n in notes)
        if f.get('spare', False) or (not moving and not reference):
            continue
        scroll_frames.append(f)
        work.append(app_work_ms(f))
        present_ms = sum(p['micros'] for p in f['phases'] if p['phase'] == 'present')/1000
        present_work.append(present_ms)
        with_present.append(app_work_ms(f)+present_ms)
        wait_work.append(sum(p['micros'] for p in f['phases'] if p['phase'] == 'wait')/1000)
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
    displayed_ids = {r[2] for r in shown if r[1]*1000-anchor <= (events[-1][0] if events else 0)+4*period}
    # A canceled submission can coincide with the same visible gap. Count
    # actual late presentation intervals once; keep canceled submissions as
    # a separate diagnostic rather than counting the missed refresh twice.
    causes = Counter(r['cause'] for r in late_records if r['kind']=='late')
    skipped_causes = Counter(r['cause'] for r in late_records if r['kind']=='skipped')
    app_late = sum(causes[c] for c in ('app', 'gpu', 'app_pacing'))
    total = len(gaps)
    return dict(frames=len(frames), sparse=sparse,
                expected_pixel_updates=len(motion_windows(events,period,scale,merge=False)),
                displayed_motion_frames=sum(f['frame'] in displayed_ids for f in scroll_frames), spare_frames=sum(f.get('spare',False) for f in frames),
                spare_work_ms=spread([app_work_ms(f) for f in frames if f.get('spare',False)]),
                spare_total_ms=sum(app_work_ms(f) for f in frames if f.get('spare',False)),
                rows_prefetched=sum(f.get('notes',[]).count('list built row') for f in frames if f.get('spare',False)), scroll_frames=len(scroll_frames), active_intervals=len(gaps),
                late_frames=sum(r['kind']=='late' for r in late_records), skipped_drawables=len(skipped),
                late_pct=100*sum(r['kind']=='late' for r in late_records)/total if total else None,
                presentation_incident_pct=100*len(late_records)/(total+len(skipped)) if total+len(skipped) else None,
                skipped_causes=dict(skipped_causes),
                app_late_frames=app_late, app_late_pct=100*app_late/total if total else None,
                system_late_frames=causes['system'], unattributed_late_frames=causes['unattributed'],
                late_attributions=late_records, interval_ms=spread(gaps), idle_intervals=len(idle),
                idle_interval_ms=spread(idle), main_over_4ms=sum(v>4 for v in work),
                main_work_ms=spread(work), main_with_present_ms=spread(with_present),
                present_ms=spread(present_work), wait_ms=spread(wait_work), gpu_p99_ms=percentile(gpu, .99),
                instructions=spread(instructions), copied_frames=copied, repainted_frames=repainted,
                picture_frames=pictures, copy_pct=100*copied/max(1,len(scroll_frames)),
                repaint_pct=100*repainted/max(1,len(scroll_frames)), blockers=dict(blockers), rows_built_in_scroll_frames=rows, rss_kib=rss)


def failures(metrics, args):
    errors = []
    if metrics['active_intervals'] < args.min_intervals and not metrics['sparse']:
        errors.append('insufficient actual presentation feedback during motion')
    if metrics['sparse'] and metrics['displayed_motion_frames'] < .8*metrics['expected_pixel_updates']:
        errors.append('missing sparse pixel-motion updates')
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


def collect(pid, future, begin, out, target):
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
        mark = request(pid, 'mark', name='drain')['data']
        mark['target'] = request(pid,'node',target=target)['data']
        polls.append(mark)
        if future.done():
            future.result()
            break
        time.sleep(1)
    return sorted(frames.values(), key=lambda f: f['frame']), polls


def run_cell(args, scene, gesture, size, hz, scale, background, repeat, present, unavailable=None, real_reference=False):
    label = f'{scene}-{gesture}-{size}-{hz}hz-{scale}x-{background}-p{present}-{repeat}'
    out = args.output / label
    out.mkdir(parents=True, exist_ok=False)
    result = dict(scene=scene, gesture=gesture, size=size, hz=hz, scale=scale,
                  background=background, repeat=repeat, present=present, status='invalid', failures=[])
    process = None
    try:
        if unavailable:
            raise RuntimeError(unavailable)
        with tempfile.TemporaryDirectory(prefix='zgx-scroll-') as tmp:
            stage = Path(tmp)
            real_app = scene == 'zeron' or real_reference
            source = ROOT / 'crates/zgx-script/examples' / ('transcript_scroll' if scene in ('transcript', 'tables') else 'scroll_scenes')
            if real_app:
                source = args.zeron_source / 'apps/zgx/ui'
                if not source.is_dir():
                    source = args.zeron_source / 'ui'
                shutil.copytree(source, stage/'app')
                shutil.copy2(ROOT/'scripts/perf/zeron_fixture.luau',stage/'app/scroll_fixture.luau')
            else:
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
            env = {k: v for k, v in os.environ.items() if not k.startswith(('ZGX_', 'ZGUI_', 'ZERON_'))}
            env.update(ZGX_SCROLL_BENCH='1', ZGX_PRESENT_DRAWABLE=present)
            if real_app:
                home = stage/'home'
                home.mkdir()
                data = stage/'data'
                data.mkdir()
                with socket.socket() as port:
                    port.bind(('127.0.0.1',0))
                    ipc = str(port.getsockname()[1])
                env.update(HOME=str(home), ZERON_DATA_DIR=str(data), ZERON_IPC_PORT=ipc,
                           ZERON_ZGX_ENGINE_PORT=ipc, ZERON_ZGX_ENGINE_DATA=str(data),
                           ZERON_ZGX_ENGINE_HOME=str(home), ZERON_ZGX_ENGINE_SPAWN='0',
                           ZERON_ZGX_NO_BROWSER='1', ZERON_ZGX_UI=str(stage/'app'),
                           ZERON_ZGX_FRAME=f'100,100,{width*scale},{height*scale},{args.native_scale}')
                if real_reference:
                    env['ZGX_SCROLL_REFERENCE']='1'
            if args.damage_check:
                env['ZGX_DAMAGE_CHECK'] = '1'
                env['ZGX_DAMAGE_DUMP'] = str(out.resolve())
            with (out / 'native.log').open('w') as log:
                executable = args.zeron_runner if real_app else (args.reference_runner if scene in ('idle','zeron-idle') else args.runner)
                argv = [str(executable),'fixture=scroll_fixture.luau','--inspect'] if real_app else [str(executable),'--inspect',str(stage/'app'),scene,str(hz)]
                process = subprocess.Popen(argv,
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
                if scene in ('idle','zeron-idle'):
                    # The system floor must not include the app's optional
                    # cadence-holding policy; that policy is tested in scenes.
                    request(process.pid, 'switch', name='refresh_grid', on=False)
                    result['reference_switches'] = {'refresh_grid': False}
                time.sleep(args.warmup)
                errors = request(process.pid, 'errors')['text']
                if 'error:' in errors or 'types:' in errors:
                    raise RuntimeError(errors)
                if real_app:
                    request(process.pid,'command',id='many')
                    request(process.pid,'command',id='bench.dark')
                    request(process.pid,'command',id='bench.'+background)
                    scroll = request(process.pid,'node',target='#transcript')['data']['scroll']
                    limit = float(re.search(r'of 0 to ([0-9.]+)',scroll).group(1))
                    request(process.pid,'set',name='bench_offset',value=limit*.35)
                else:
                    request(process.pid, 'set', name='translucent', value=background == 'translucent')
                reset_offset = 90000
                if gesture == 'stream':
                    if real_app:
                        request(process.pid,'set',name='bench_offset',value=1e9)
                        request(process.pid,'command',id='bench.reset')
                    else:
                        request(process.pid, 'set', name='offset', value=1e9)
                    time.sleep(.5)
                    scroll = request(process.pid, 'node', target='#transcript')['data']['scroll']
                    limit = float(re.search(r'of 0 to ([0-9.]+)', scroll).group(1))
                    reset_offset = max(0, limit - 4000)
                    if real_app:
                        request(process.pid,'set',name='bench_offset',value=reset_offset)
                if real_app:
                    request(process.pid,'command',id='bench.reset')
                else:
                    request(process.pid, 'set', name='offset', value=reset_offset)
                time.sleep(.5)
                # Preserve the actual loaded tree and pixels before timing.
                tree = request(process.pid, 'tree', depth=2)
                (out/'tree.json').write_text(json.dumps(tree,indent=2)+'\n')
                snapshot = request(process.pid, 'snapshot', target='#shell' if real_app else target, width=2200)['data']
                shutil.copy2(snapshot['path'], out/'scene.png')
                if real_app:
                    state = request(process.pid,'state')['data']
                    (out/'fixture_state.json').write_text(json.dumps(state,indent=2)+'\n')
                    result['fixture_state'] = {k:v for k,v in state.items() if 'bench_' in k}
                # Visit a long fling's range before measuring the warmed path.
                warm = tape('fling', hz) if scene not in ('idle','zeron-idle') else '# zgx wheel tape 1\n0 pixel 0 0 started\n100 pixel 0 0 ended\n'
                request(process.pid, 'replay', target='#transcript', tape=warm, timeout=15)
                if real_app:
                    request(process.pid,'command',id='bench.reset')
                else:
                    request(process.pid, 'set', name='offset', value=reset_offset)
                if scene == 'nested':
                    request(process.pid, 'set', name='horizontal_offset', value=1000)
                time.sleep(.5)
                if gesture == 'stream':
                    if scene not in ('transcript','zeron'):
                        raise RuntimeError('stream gesture requires transcript scene')
                    if real_app:
                        request(process.pid,'command',id='bench.stream')
                    else:
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
                    if scene in ('idle','zeron-idle'):
                        # A clock reference must not inject even zero-delta
                        # began/ended events, which can disturb host pacing.
                        future = pool.submit(time.sleep,events[-1][0]/1000)
                        result['reference_no_input']=True
                    else:
                        future = pool.submit(request, process.pid, 'replay', target=target, tape=text,
                                             timeout=events[-1][0] / 1000 + 20)
                    try:
                        frames, polls = collect(process.pid, future, begin, out, target)
                    except Exception:
                        process.terminate()
                        process.wait(timeout=5)
                        raise
                    result['replay'] = future.result()
                errors = request(process.pid,'errors')['text']
                if 'error:' in errors or 'types:' in errors:
                    raise RuntimeError(errors)
                after = rss(process.pid)
                result['polls'] = polls
                result['frames'] = frames
                # Give asynchronous presentation callbacks time to arrive.
                time.sleep(.3)
                presents = []
                replay_starts = []
                latency = []
                gpu_by_frame = Counter()
                observed_frames = {f['frame'] for f in frames}
                sample_first = min(observed_frames, default=0)
                sample_last = max(observed_frames, default=0)
                log.flush()
                for line in (out / 'native.log').read_text().splitlines():
                    if line.startswith('SCROLL_GPU '):
                        _, frame_id, micros = line.split()
                        gpu_by_frame[int(frame_id)] += float(micros)
                    if line.startswith('SCROLL_LATENCY '):
                        latency.append(tuple(map(float, line.split()[1:])))
                    if line.startswith('SCROLL_REPLAY '):
                        replay_starts.append(int(line.split()[1]) / 1e6)
                    match = re.fullmatch(r'SCROLL_PRESENT ([0-9.]+) ([0-9.]+) ([0-9]+) ([0-9.]+)', line)
                    if match:
                        record = tuple(map(float, match.groups()))
                        if not sample_first <= record[2] <= sample_last:
                            continue
                        presents.append(record)
                        if record[3] > 0 and record[1] > 0:
                            latency.append((record[3], record[1]))
                for f in frames:
                    f['trace_gpu_micros'] = f.get('gpu_micros')
                    f['gpu_micros'] = gpu_by_frame.get(f['frame'])
                result['gpu_completion_micros'] = dict(gpu_by_frame)
                result['presents'] = presents
                if scene in ('idle','zeron-idle'):
                    replay_starts=[begin['host_ns']/1e6]
                if not replay_starts:
                    raise RuntimeError('missing replay clock anchor')
                result['replay_start_host_ms'] = replay_starts[-1]
                metrics = summarize(frames, presents, events, hz, scale, replay_starts[-1],
                                    dict(before=before, after=after, delta=after-before),
                                    reference=scene in ('idle','zeron-idle'), main_budget=args.max_main_ms, gpu_budget=args.max_gpu_ms,
                                    sparse=gesture in ('slow','notches'))
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
                process.terminate()
                process.wait(timeout=5)
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
    return {k:v for k,v in result.items() if k not in ('frames', 'presents', 'latency_samples', 'polls', 'gpu_completion_micros')}


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


class RefreshModes:
    """Select actual modes of the same logical/physical size; restore on exit."""
    def __init__(self, output):
        self.binary = ROOT / 'target/scroll-display'
        source = ROOT / 'scripts/perf/scroll_display.swift'
        if not self.binary.exists() or self.binary.stat().st_mtime < source.stat().st_mtime:
            subprocess.run(['swiftc', '-O', str(source), '-o', str(self.binary)], check=True)
        self.original = json.loads(subprocess.check_output([str(self.binary), 'status'], text=True))
        self.output = output
        self.actions = []
        self.current = self.original['current']['id']

    def select(self, hz):
        original = self.original['current']
        same_size = [m for m in [original, *self.original['modes']] if all(m[k] == original[k]
                     for k in ('width', 'height', 'pixel_width', 'pixel_height'))]
        matches = [m for m in same_size if abs(m['hz']-hz) < .5]
        # ProMotion's default mode reports 0 (adaptive). Returning to that
        # exact original mode is allowed; inspect still verifies the real rate.
        if not matches and original['hz'] == 0 and hz == 120:
            matches = [original]
        if not matches:
            raise RuntimeError(f'no actual {hz} Hz mode at the original display size')
        chosen = matches[0]['id']
        if chosen != self.current:
            response = json.loads(subprocess.check_output([str(self.binary), 'set', str(chosen)], text=True))
            self.actions.append(response)
            self.current = chosen
            time.sleep(1)

    def restore(self):
        original = self.original['current']['id']
        try:
            if self.current != original:
                subprocess.run([str(self.binary), 'set', str(original)], check=True)
        finally:
            (self.output / 'display-modes.json').write_text(json.dumps(
                dict(original=self.original, actions=self.actions), indent=2)+'\n')


def native_floor(args, size, hz, scale, present, label):
    """Independent CADisplayLink/Metal window; no zgx pacing in this process."""
    out=args.output/label
    out.mkdir()
    mode='tiles' if present=='0' else 'drawables'
    width,height=SIZES[size]
    native_size=[width*scale/args.native_scale,height*scale/args.native_scale]
    binary=ROOT/'target/scroll-native'
    with (out/'native.log').open('w') as log:
        subprocess.run([str(binary),*map(str,native_size),str(hz),mode,'10'],
                       stdout=log,stderr=log,check=True,timeout=30)
    begin=end=None
    wakes={}; gpu=[]; work=[]; presents=[]; unavailable=0
    for line in (out/'native.log').read_text().splitlines():
        parts=line.split()
        if not parts: continue
        if parts[0]=='NATIVE_BEGIN': begin=float(parts[1])
        elif parts[0]=='NATIVE_END': end=float(parts[1])
        elif parts[0]=='NATIVE_WAKE': wakes[int(parts[1])]=list(map(float,parts[2:]))
        elif parts[0]=='NATIVE_PRESENT': presents.append(list(map(float,parts[1:])))
        elif parts[0]=='NATIVE_GPU': gpu.append((int(parts[1]),float(parts[2])))
        elif parts[0]=='NATIVE_WORK': work.append((int(parts[1]),float(parts[2])))
        elif parts[0]=='NATIVE_UNAVAILABLE': unavailable+=1
    if begin is None or end is None: raise RuntimeError('native floor missing sample anchors')
    sample={n:v for n,v in wakes.items() if begin<=v[0]<=end}
    presented=sorted(p[2] for p in presents if int(p[0]) in sample and p[2]>0)
    gaps=[1000*(b-a) for a,b in zip(presented,presented[1:])]
    skipped=sum(p[2]<=0 for p in presents if int(p[0]) in sample)
    late=sum(gap>1.5*1000/hz for gap in gaps)
    m=dict(mode=mode,native_points=native_size,requested_scale=scale,hz=hz,
           late_frames=late,skipped_drawables=skipped,late_pct=100*late/max(1,len(gaps)),
           interval_ms=spread(gaps),main_work_ms=spread([v for n,v in work if n in sample]),
           gpu_p99_ms=percentile([v for n,v in gpu if n in sample],.99),
           wake_after_timestamp_ms=spread([1000*(v[0]-v[1]) for v in sample.values()]),
           wake_after_target_ms=spread([1000*(v[0]-v[2]) for v in sample.values()]),
           display_period_ms=spread([1000*(v[2]-v[1]) for v in sample.values()]),
           samples=len(presented),unavailable_drawables=unavailable,
           binary_sha256=sha(binary),source_sha256=sha(ROOT/'scripts/perf/scroll_native.swift'))
    if len(presented)<hz*8: raise RuntimeError('native floor lacks sustained feedback')
    if abs(m['display_period_ms']['p50']-1000/hz)>.5:
        raise RuntimeError('native display clock differs from requested refresh')
    if m['gpu_p99_ms'] is None or m['main_work_ms']['max'] is None:
        raise RuntimeError('native floor lacks work/GPU evidence')
    (out/'run.json').write_text(json.dumps(m,indent=2)+'\n')
    return m


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('command', choices=['run', 'headless', 'reference'])
    p.add_argument('--runner', type=Path, default=ROOT / 'target/dist/zgx-run')
    p.add_argument('--zeron-source',type=Path,default=Path.home()/'Documents/GitHub/zeron-scrollbench')
    p.add_argument('--zeron-runner',type=Path,default=ROOT/'target/zeron-native/current/zeron-zgx')
    p.add_argument('--reference-runner', type=Path, default=ROOT / 'target/dist/examples/scroll_reference')
    p.add_argument('--output', type=Path, default=ROOT / 'target/scroll-results' / time.strftime('%Y%m%d-%H%M%S'))
    p.add_argument('--scenes', default='transcript,plain,grid,nested')
    p.add_argument('--gestures', default='slow,flick,reversals,notches,fling,wing,stream')
    p.add_argument('--sizes', default='S,M,L')
    p.add_argument('--hz', default='120')
    p.add_argument('--native-floor',action='store_true',help='bracket each zgx idle reference with independent Swift/Metal references')
    p.add_argument('--set-refresh', action='store_true', help='select matching native display modes and restore the original')
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
    args.reference_runner = args.reference_runner.resolve()
    args.zeron_source = args.zeron_source.resolve()
    args.zeron_runner = args.zeron_runner.resolve()
    if 'zeron' in args.scenes.split(',') and not args.zeron_runner.is_file():
        p.error('build the real app with prepare_zeron.py and supply --zeron-runner')
    if not args.reference_runner.is_file():
        p.error('build scroll_reference with --profile dist --features inspect first')
    if args.native_floor:
        source=ROOT/'scripts/perf/scroll_native.swift'
        binary=ROOT/'target/scroll-native'
        if not binary.exists() or binary.stat().st_mtime<source.stat().st_mtime:
            subprocess.run(['swiftc','-O',str(source),'-o',str(binary)],check=True)
    args.output.mkdir(parents=True, exist_ok=False)
    audit = dict(commit=subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip(),
                 diff=subprocess.check_output(['git', 'diff', 'HEAD'], cwd=ROOT, text=True),
                 runner=str(args.runner), runner_sha256=sha(args.runner),
                 reference_sha256=sha(args.reference_runner), platform=platform.platform(),
                 machine=platform.machine(), args={k: str(v) if isinstance(v, Path) else v for k,v in vars(args).items()})
    sources = args.output / 'sources'
    sources.mkdir()
    for name in ('transcript_scroll', 'scroll_scenes'):
        shutil.copytree(ROOT / 'crates/zgx-script/examples' / name, sources / name)
    shutil.copy2(Path(__file__), sources / 'scroll.py')
    shutil.copy2(ROOT/'scripts/perf/zeron_fixture.luau',sources/'zeron_fixture.luau')
    if args.zeron_runner.is_file():
        audit['zeron_binary_sha256']=sha(args.zeron_runner)
        audit['zeron_source_commit']=subprocess.check_output(['git','rev-parse','HEAD'],cwd=args.zeron_source,text=True).strip()
    shutil.copy2(ROOT / 'scripts/perf/scroll_native.swift', sources / 'scroll_native.swift')
    shutil.copy2(ROOT / 'scripts/perf/scroll_display.swift', sources / 'scroll_display.swift')
    shutil.copy2(ROOT / 'crates/zgx-script/examples/scroll_reference.rs', sources / 'scroll_reference.rs')
    audit['sources'] = {str(f.relative_to(sources)): sha(f) for f in sources.rglob('*') if f.is_file()}
    audit['rustc'] = subprocess.check_output(['rustc', '-vV'], text=True)
    audit['displays'] = subprocess.check_output(['system_profiler', 'SPDisplaysDataType'], text=True)
    (args.output / 'audit.json').write_text(json.dumps(audit, indent=2) + '\n')
    results = []
    references = {}
    if args.command=='reference':
        args.gestures='reference'
    cells = itertools.product(range(args.repeats), args.scenes.split(','), args.gestures.split(','),
                              args.sizes.split(','), map(int, args.hz.split(',')), map(float, args.scale.split(',')),
                              args.backgrounds.split(','), args.present.split(','))
    refresh = RefreshModes(args.output) if args.set_refresh else None
    try:
        for repeat, scene, gesture, size, hz, scale, bg, present in cells:
            if scene not in SCENES or gesture not in (*GESTURES,'reference') or size not in SIZES or present not in ('0', '1', 'auto'):
                p.error('unknown scene, gesture, or size')
            if gesture == 'stream' and scene not in ('transcript','zeron'):
                continue
            unavailable = None
            if refresh is not None:
                try:
                    refresh.select(hz)
                except RuntimeError as error:
                    unavailable = str(error)
            if unavailable:
                results.append(run_cell(args, scene, gesture, size, hz, scale, bg, repeat, present, unavailable))
                (args.output / 'summary.md').write_text(report(results))
                (args.output / 'summary.json').write_text(json.dumps(results, indent=2)+'\n')
                continue
            print(f'{scene} {gesture} {size} {hz}Hz {scale}x {bg} trial {repeat+1}', flush=True)
            key = (size, hz, scale, bg, present, repeat, 'zeron' if scene=='zeron' else 'generic')
            if key not in references:
                floor_before=native_floor(args,size,hz,scale,present,f'native-before-{len(references)}') if args.native_floor else None
                ref = run_cell(args, 'zeron-idle' if scene=='zeron' else 'idle', 'reference', size, hz, scale, bg, repeat, present, real_reference=scene=='zeron')
                if args.native_floor:
                    ref['native_floor_before']=floor_before
                    ref['native_floor_after']=native_floor(args,size,hz,scale,present,f'native-after-{len(references)}')
                references[key] = ref
                results.append(ref)
            if args.command=='reference':
                (args.output/'summary.md').write_text(report(results))
                (args.output/'summary.json').write_text(json.dumps(results,indent=2)+'\n')
                continue
            r = run_cell(args, scene, gesture, size, hz, scale, bg, repeat, present)
            r['reference'] = references[key].get('metrics')
            if references[key]['status'] != 'pass':
                r['failures'].append('idle reference failed: system floor cannot be certified')
                r['status'] = 'fail'
            results.append(r)
            print(r['status'], '; '.join(r['failures']), flush=True)
            (args.output / 'summary.md').write_text(report(results))
            (args.output / 'summary.json').write_text(json.dumps(results, indent=2) + '\n')
    finally:
        if refresh is not None:
            refresh.restore()
    print(report(results))
    return int(any(r['status'] != 'pass' for r in results))


if __name__ == '__main__':
    sys.exit(main())
