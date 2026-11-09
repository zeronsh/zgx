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
            # Orbit near the streaming tail instead of pushing into its edge.
            dy = 2 if (i // (hz//2)) % 2 == 0 else -2
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


def in_motion_window(frame, windows, trace_to_host_ms, anchor):
    if trace_to_host_ms is None or 'ended_ms' not in frame:return False
    end=trace_to_host_ms+frame['ended_ms']-anchor
    start=end-frame.get('frame_micros',0)/1000
    return any(start<=b and end>=a for a,b in windows)


def workload(frames):
    groups={name:[] for name in ('paint','encode','present','wait','rest')}
    for frame in frames:
        phases=Counter()
        for phase in frame['phases']:phases[phase['phase']]+=phase['micros']/1000
        for name in groups:
            groups[name].append(sum(v for k,v in phases.items() if k not in ('paint','encode','present','wait')) if name=='rest' else phases[name])
    return dict(frames=len(frames),main_ms=spread([app_work_ms(f) for f in frames]),
                main_over_4ms=sum(app_work_ms(f)>4 for f in frames),
                rows_built=sum(f.get('notes',[]).count('list built row') for f in frames),
                gpu_ms=spread([f['gpu_micros']/1000 for f in frames if f.get('gpu_micros') is not None]),
                phases_ms={name:dict(**spread(values),mean=sum(values)/len(values) if values else None,total=sum(values),over4ms=sum(v>4 for v in values)) for name,values in groups.items()},
                worst_main_frame=max(frames,key=app_work_ms,default=None))


def material_matches(state, background):
    return state.get('bench_effective_frost') is (background=='translucent')


def frame_timing_notes(frame):
    acquisition=[]; gpu_wait=[]; handovers=[]
    for note in frame.get('notes',[]):
        match=re.fullmatch(r'next drawable waited ([0-9.]+) us',note)
        if match:acquisition.append(float(match[1]))
        match=re.fullmatch(r'waited ([0-9.]+) us for the GPU',note)
        if match:gpu_wait.append(float(match[1]))
        match=re.fullmatch(r'(handed over|submitted drawable) at (\d+) ns in ([0-9.]+) us',note)
        if match:
            handovers.append(dict(kind='drawable' if match[1]=='submitted drawable' else 'layer',
                                  end_ns=int(match[2]),micros=float(match[3])))
    return dict(acquisition_micros=sum(acquisition) if acquisition else None,
                gpu_wait_micros=sum(gpu_wait) if gpu_wait else None,handovers=handovers)


def presentation_timings(frames, records, anchor, trace_to_host_ms):
    """Join same-frame timing notes; never substitute paint time for acquisition."""
    by_id={frame['frame']:frame for frame in frames}
    notes={index:frame_timing_notes(frame) for index,frame in by_id.items()}
    samples=[]
    for callback,shown,index,started in records:
        if shown<=0 or shown*1000<anchor or index not in by_id:continue
        timing=notes[index]; preferred='drawable' if started>0 else 'layer'
        candidates=[value for value in timing['handovers'] if value['kind']==preferred]
        if not candidates and preferred=='drawable':candidates=timing['handovers']
        value=candidates[0] if len(candidates)==1 else None
        sample=dict(frame=index,shown_s=shown,acquisition_micros=timing['acquisition_micros'],
                    gpu_wait_micros=timing['gpu_wait_micros'],handover=value,
                    clock_aligned=None,present_to_display_ms=None,handover_end_to_display_ms=None)
        if value is not None:
            # N is the API return; its measured duration reconstructs the
            # start to within the note's one-microsecond quantization.
            begin=value['end_ns']-value['micros']*1000
            frame=by_id[index]
            aligned=begin<=callback*1e9+2e6 and shown*1e9>=begin-1e4 and shown<=callback+.002
            if started>0:aligned=aligned and begin>=started*1e9-2e6
            if trace_to_host_ms is not None and 'ended_ms' in frame:
                end=(trace_to_host_ms+frame['ended_ms'])*1e6
                start=end-frame.get('frame_micros',0)*1000
                aligned=aligned and start-2e6<=begin<=value['end_ns']<=end+2e6
            sample['clock_aligned']=aligned
            if aligned:
                sample['present_to_display_ms']=(shown*1e9-begin)/1e6
                sample['handover_end_to_display_ms']=(shown*1e9-value['end_ns'])/1e6
        samples.append(sample)
    return dict(samples=samples,
                acquisition_ms=spread([v['acquisition_micros']/1000 for v in notes.values() if v['acquisition_micros'] is not None]),
                gpu_wait_ms=spread([v['gpu_wait_micros']/1000 for v in notes.values() if v['gpu_wait_micros'] is not None]),
                present_to_display_ms=spread([s['present_to_display_ms'] for s in samples if s['present_to_display_ms'] is not None]),
                handover_end_to_display_ms=spread([s['handover_end_to_display_ms'] for s in samples if s['handover_end_to_display_ms'] is not None]),
                coverage=dict(shown_records=len(samples),handover_records=sum(s['handover'] is not None for s in samples),
                              missing_handover=sum(s['handover'] is None for s in samples),
                              clock_mismatch=sum(s['clock_aligned'] is False for s in samples)))


def verify_environment_text(text, overrides):
    return {item:bool(re.search(r'(?:^|\s)'+re.escape(item)+r'(?=\s|$)',text)) for item in overrides}


def behavior_checks(metrics, overrides, presents, present, reference, require_source=False, expected_source=None):
    checks={}
    if 'ZGX_HANDOVER_ASAP=1' in overrides:
        checks['asap_no_hold']=metrics['wait_ms']['p99'] is not None and metrics['wait_ms']['p99']<=.5
    if 'ZGX_VIEW_CLOCK=1' in overrides and present=='0' and (reference or 'ZGX_SCROLL_LINK=1' in overrides):
        checks['view_clock_no_align_hold']=metrics['wait_ms']['p99'] is not None and metrics['wait_ms']['p99']<=.5
    if 'ZGX_SCROLL_DRAWABLES=1' in overrides and present=='0':
        exact=sum(len(r)>3 and r[1]>0 and r[3]>0 for r in presents)
        checks['hybrid_tiles_at_rest' if reference else 'hybrid_drawables_during_scroll']=exact==0 if reference else exact>=10
    flags=dict(value.split('=',1) for value in overrides)
    pacing=any(flags.get(key)=='1' for key in ('ZGX_HANDOVER_ASAP','ZGX_VIEW_CLOCK','ZGX_SCROLL_LINK')) or 'ZGX_LEAD_US' in flags
    counts=metrics.get('frame_source_counts',{})
    if pacing or require_source or expected_source:
        timer=flags.get('ZGX_HANDOVER_ASAP')=='1' or 'ZGX_LEAD_US' in flags or (not reference and flags.get('ZGX_SCROLL_LINK')!='1')
        expected='timer on the grid' if timer else 'view link' if flags.get('ZGX_VIEW_CLOCK')=='1' else 'display link'
        if not timer and (present=='1' or (present=='auto' and any(len(r)>3 and r[3]>0 for r in presents)) or (flags.get('ZGX_SCROLL_DRAWABLES')=='1' and not reference)):
            expected='view link'
        if expected_source is not None:expected=expected_source
        total=sum(counts.values())
        checks['frame_source_is_'+expected]=total>=10 and total>=.8*metrics.get('render_frames',metrics.get('scroll_frames',0)) and counts.get(expected,0)>=.95*total
    return checks


def host_environment(overrides, present, inherited=None):
    source=os.environ if inherited is None else inherited
    env={k:v for k,v in source.items() if not k.startswith(('ZGX_','ZGUI_','ZERON_'))}
    env.update(ZGX_SCROLL_BENCH='1',ZGX_PRESENT_DRAWABLE=present)
    env.update(dict(value.split('=',1) for value in overrides))
    return env


def picture_costs(frames):
    groups = {name: [] for name in ('promotion', 'raster', 'composite_only')}
    for frame in frames:
        if frame.get('spare', False):continue
        notes = frame.get('notes', [])
        promotion = any(' made a picture' in note for note in notes)
        raster = any(re.search(r' rastered \d+ px rows', note) for note in notes)
        composite = any('composited its picture' in note for note in notes)
        if promotion:groups['promotion'].append(frame)
        if raster:groups['raster'].append(frame)
        if composite and not promotion and not raster:groups['composite_only'].append(frame)
    result = {}
    for name, selected in groups.items():
        worst = max(selected, key=app_work_ms, default=None)
        result[name] = dict(count=len(selected), main_ms=spread([app_work_ms(f) for f in selected]),
                            gpu_ms=spread([f['gpu_micros']/1000 for f in selected if f.get('gpu_micros') is not None]),
                            worst_main_frame=worst)
    return result


def coalesce_presentations(records, period):
    """Keep one displayed update per refresh; retain duplicate submissions."""
    groups=[]
    for record in sorted(records,key=lambda r:r[1]):
        if groups and (record[1]-groups[-1][-1][1])*1000 < period/2:
            groups[-1].append(record)
        else:
            groups.append([record])
    shown=[max(group,key=lambda r:(r[2],r[0])) for group in groups]
    return shown,groups


def summarize(frames, presents, events, hz, scale, anchor, rss, *, reference=False,
              main_budget=4., gpu_budget=4., sparse=False, trace_to_host_ms=None):
    period = 1000 / hz
    windows = [(0, events[-1][0])] if reference and events else motion_windows(events, period, scale)
    # Feedback carries the exact trace index captured before presenting. An
    # older runner's two-field record remains readable but cannot certify
    # attribution: missing frame identity is an explicit gate failure.
    records = [(r[0], r[1], int(r[2]) if len(r) > 2 else 0,
                r[3] if len(r) > 3 else 0) for r in presents]
    shown,display_groups = coalesce_presentations([r for r in records if r[1]>0 and r[1]*1000>=anchor],period)
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

    double_then_skip=0
    for group_index,(prev, current) in enumerate(zip(shown, shown[1:])):
        a, b = prev[1]*1000-anchor, current[1]*1000-anchor
        if not active_gap(a, b, windows, period):
            idle.append(b-a)
            continue
        gaps.append(b-a)
        if b-a > 1.5*period:
            double_then_skip += len({int(r[2]) for r in display_groups[group_index]})>1
            ids = range(prev[2], current[2]+1) if prev[2] and current[2] >= prev[2] else []
            record=attribute('late', b, ids)
            record.update(previous_presented_ms=a,interval_ms=b-a)
            late_records.append(record)
    skipped = [r for r in records if r[1] <= 0 and r[0]*1000 >= anchor]
    for r in skipped:
        late_records.append(attribute('skipped', r[0]*1000-anchor, [r[2]] if r[2] else []))
    copied = repainted = pictures = rows = 0
    blockers = Counter()
    gpu, instructions, work, scroll_frames = [], [], [], []
    with_present, present_work, wait_work = [], [], []
    motion_frames=[];noted_frames=0;frame_sources=Counter()
    for f in frames:
        notes = f.get('notes', [])
        moving = any('copied its pixels' in n or 'repainted its view:' in n or
                     'composited its picture' in n for n in notes)
        in_motion=in_motion_window(f,windows,trace_to_host_ms,anchor)
        if f.get('spare', False) or (not moving and not reference and not in_motion):
            continue
        frame_sources.update(n.removeprefix('frame source: ') for n in notes if n.startswith('frame source: '))
        noted_frames+=moving or reference
        if in_motion:motion_frames.append(f)
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
    render_estimates=[]; end_estimates=[]; estimate_errors=[]
    if trace_to_host_ms is not None:
        for r in shown:
            f=by_id.get(r[2])
            if f is None or 'ended_ms' not in f:continue
            ended=trace_to_host_ms+f['ended_ms']
            start=ended-f.get('frame_micros',0)/1000
            render_estimates.append(r[1]*1000-start)
            end_estimates.append(r[1]*1000-ended)
            if r[3]>0:estimate_errors.append(r[3]*1000-start)
    causes = Counter(r['cause'] for r in late_records if r['kind']=='late')
    skipped_causes = Counter(r['cause'] for r in late_records if r['kind']=='skipped')
    app_late = sum(causes[c] for c in ('app', 'gpu', 'app_pacing'))
    total = len(gaps)
    duration=events[-1][0] if events else 0
    shown_in_tape=[r for r in shown if 0<=r[1]*1000-anchor<=duration]
    return dict(frames=len(frames), sparse=sparse,
                presentation_timings=presentation_timings(scroll_frames,records,anchor,trace_to_host_ms),
                picture_costs=picture_costs(frames),
                motion_window_work=workload(motion_frames),
                work_selection="motion-window frame intervals plus noted scroll frames; spare excluded",
                noted_scroll_frames=noted_frames,frame_source_counts=dict(frame_sources),
                render_frames=sum(f.get('gpu_micros') is not None for f in scroll_frames),
                worst_main_frame=max(scroll_frames,key=app_work_ms,default=None),
                shown_frames_tape=len(shown_in_tape),
                shown_frames_motion=sum(any(a<=r[1]*1000-anchor<=b for a,b in windows) for r in shown_in_tape),
                tape_duration_ms=duration,
                same_refresh_extra_frames=sum(max(0,len({r[2] for r in g})-1) for g in display_groups),
                duplicate_feedback_records=sum(len(g)-len({r[2] for r in g}) for g in display_groups),
                double_then_skip_intervals=double_then_skip,
                render_start_to_shown_ms_estimate=spread(render_estimates),
                frame_end_to_shown_ms_estimate=spread(end_estimates),
                render_start_estimate_error_ms=spread(estimate_errors),
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
    if metrics.get('noted_scroll_frames',metrics['scroll_frames']) < 10:
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
                  background=background, repeat=repeat, present=present, status='invalid', failures=[],
                  engine_pin=args.engine_pin,engine_commit=args.engine_commit)
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
                if getattr(args,'reset_row',0)>0:
                    transcript=stage/'app/ds/transcript.luau'
                    source_text=transcript.read_text()
                    hook='on_mount = function(_el)\n'
                    if source_text.count(hook)!=1:
                        raise RuntimeError('unrecognized transcript mount hook for deterministic row reset')
                    source_text=source_text.replace(hook,hook+"""                        -- Benchmark-only control; rendering and list data stay real.
                        require("@app/scroll_control").go=function(row: number): boolean
                            local el=list:peek()
                            return el~=nil and el:scroll_to_row(row,{align="start"})
                        end
""")
                    control='return { go=function(_row: number): boolean return false end }\n'
                    (stage/'app/scroll_control.luau').write_text(control)
                    fixture=stage/'app/scroll_fixture.luau'
                    fixture_text=fixture.read_text().replace('return ui.overlay {', """local reset_control=require("./scroll_control")
local bench_row=ui.shared("bench.reset_row",160)
local bench_ok=ui.shared("bench.goto_ok",false)
ui.command { id="bench.goto_row",run=function() bench_ok:set(reset_control.go(bench_row:peek())) end }
return ui.overlay {""")
                    fixture.write_text(fixture_text)
                    (out/'scroll-fixture.luau').write_text(fixture_text)
                    (out/'scroll-control.luau').write_text(control)
                    transcript.write_text(source_text)
                    (out/'transcript-fixture.luau').write_text(source_text)

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
            tracked_target='#horizontal-content' if scene=='nested' else target
            result.update(replay_target=target,tracked_target=tracked_target)
            (out / 'gesture.tape').write_text(text)
            events = parse_tape(text)
            result['tape_sha256'] = hashlib.sha256(text.encode()).hexdigest()
            env = host_environment(args.host_env,present)
            result['host_env']=dict(v.split('=',1) for v in args.host_env)
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
                if args.host_env:
                    actual_env=subprocess.check_output(['ps','eww','-p',str(process.pid)],text=True)
                    result['host_env_verified']=verify_environment_text(actual_env,args.host_env)
                    if not all(result['host_env_verified'].values()):raise RuntimeError('explicit diagnostic flags missing from actual host process')
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
                    grid=getattr(args,'reference_refresh_grid','off')=='on'
                    request(process.pid, 'switch', name='refresh_grid', on=grid)
                    result['reference_switches'] = {'refresh_grid': grid}
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
                    if getattr(args,'reset_row',0)>0:
                        request(process.pid,'set',name='bench.reset_row',value=args.reset_row)
                else:
                    request(process.pid, 'set', name='translucent', value=background == 'translucent')
                reset_offset = 90000
                if gesture == 'stream':
                    if real_app:
                        request(process.pid,'set',name='bench_offset',value=1e9)
                        request(process.pid,'command',id='bench.reset')
                        if getattr(args,'reset_row',0)>0 and gesture!='stream':
                            request(process.pid,'command',id='bench.goto_row')
                    else:
                        request(process.pid, 'set', name='offset', value=1e9)
                    time.sleep(.5)
                    scroll = request(process.pid, 'node', target='#transcript')['data']['scroll']
                    limit = float(re.search(r'of 0 to ([0-9.]+)', scroll).group(1))
                    viewport=float(re.search(r'x([0-9.]+)$',request(process.pid,'node',target='#transcript')['data']['box']).group(1))
                    reset_offset = max(0, limit - viewport/2)
                    if real_app:
                        request(process.pid,'set',name='bench_offset',value=reset_offset)
                if real_app:
                    request(process.pid,'command',id='bench.reset')
                    if getattr(args,'reset_row',0)>0 and gesture!='stream':
                        request(process.pid,'command',id='bench.goto_row')
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
                    if not material_matches(result['fixture_state'],background):raise RuntimeError('actual shell material differs from requested background')
                if scene in ('idle','zeron-idle'):
                    # Never seed a reference with wheel state, even in warmup:
                    # gesture/coast grace periods can change the tick source.
                    result['reference_warmup']='no-wheel-events'
                    time.sleep(args.warmup)
                else:
                    # Visit a long fling's range before measuring the warmed path.
                    request(process.pid, 'replay', target='#transcript', tape=tape('fling',hz), timeout=15)
                    if real_app:
                        request(process.pid,'command',id='bench.reset')
                        if getattr(args,'reset_row',0)>0 and gesture!='stream':
                            request(process.pid,'command',id='bench.goto_row')
                    else:
                        request(process.pid, 'set', name='offset', value=reset_offset)
                if scene == 'nested':
                    request(process.pid, 'set', name='horizontal_offset', value=90000)
                time.sleep(.5)
                if gesture == 'stream':
                    if scene not in ('transcript','zeron'):
                        raise RuntimeError('stream gesture requires transcript scene')
                    if real_app:
                        request(process.pid,'command',id='bench.stream')
                    else:
                        request(process.pid, 'set', name='stream_row', value=400)
                if real_app:
                    result['setup_anchor']=request(process.pid,'state',name='chat.transcript.anchor')['data']
                    result['reset_row']=getattr(args,'reset_row',0)
                    if getattr(args,'reset_row',0)>0 and gesture!='stream':
                        ok=request(process.pid,'state',name='bench.goto_ok')['data'].get('bench.goto_ok')
                        if ok is not True:raise RuntimeError('deterministic transcript row reset failed')
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
                        frames, polls = collect(process.pid, future, begin, out, tracked_target)
                    except Exception:
                        process.terminate()
                        process.wait(timeout=5)
                        raise
                    result['replay'] = future.result()
                errors = request(process.pid,'errors')['text']
                if 'error:' in errors or 'types:' in errors:
                    raise RuntimeError(errors)
                after = rss(process.pid)
                memory = dict(before=before, after=after, delta=after-before)
                if args.rest_seconds > 0 and scene not in ('idle', 'zeron-idle'):
                    # No inspector request or synthetic input during settling.
                    # A vanished process makes the cell invalid, never zero RSS.
                    time.sleep(args.rest_seconds)
                    if process.poll() is not None:
                        raise RuntimeError('host exited during the rest-memory sample')
                    memory['rest_after'] = rss(process.pid)
                    memory['rest_seconds'] = args.rest_seconds
                    memory['rest_delta_from_before'] = memory['rest_after'] - before
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
                                    memory,
                                    reference=scene in ('idle','zeron-idle'), main_budget=args.max_main_ms, gpu_budget=args.max_gpu_ms,
                                    sparse=gesture in ('slow','notches'),
                                    trace_to_host_ms=begin['host_ns']/1e6-begin['trace_ms'])
                metrics['start_to_present_ms'] = spread([(p-s)*1000 for s,p in latency if s*1000 >= replay_starts[-1]])
                result['latency_samples'] = latency
                result['metrics'] = metrics
                result['behavior_checks']=behavior_checks(metrics,args.host_env,presents,present,scene in ('idle','zeron-idle'),getattr(args,'require_frame_source',False),getattr(args,'reference_source' if scene in ('idle','zeron-idle') else 'scroll_source',None))
                if not all(result['behavior_checks'].values()):raise RuntimeError('diagnostic arm failed observed behaviour check: '+str(result['behavior_checks']))
                result['failures'] = failures(metrics, args)
                latency_p99 = metrics['start_to_present_ms']['p99']
                if present == '1' and latency_p99 is None:
                    result['failures'].append('missing frame start-to-presentation latency')
                if latency_p99 is not None and latency_p99 > args.max_present_latency_ms:
                    result['failures'].append('frame start-to-presentation latency over budget')
                if 'ZGX_GPU_STATS=1' in args.host_env:
                    result['gpu_stats_lines']=sum(line.startswith('GPU_STATS ') for line in (out/'native.log').read_text().splitlines())
                    if not result['gpu_stats_lines']:raise RuntimeError('GPU stats requested but host stderr has no GPU_STATS lines')
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
    lines = ['| Engine | Scene | Gesture | Size | Hz / scale / bg / present | Late % | Interval p99 ms | Main >4ms | App/GPU / system / unknown late | Result |',
             '|---|---|---|---|---|---:|---:|---:|---|---|']
    for r in results:
        m = r.get('metrics', {})
        def fmt(v):
            return '—' if v is None else f'{v:.2f}'
        lines.append(f"| {(r.get('engine_pin') or r.get('engine_commit') or '—')[:8]} | {r['scene']} | {r['gesture']} | {r['size']} | {r['hz']} / {r['scale']} / {r['background']} / {r.get('present', '?')} | "
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


def native_presentation_timings(presents, wakes, handovers, acquisition, period):
    samples=[]; previous=None
    for n,callback,shown,started in sorted(presents,key=lambda value:value[2]):
        n=int(n)
        if n not in wakes or shown<=0:continue
        handover=handovers.get(n)
        aligned=None; latency=None; end_latency=None; phase=None
        if handover is not None:
            begin,end=handover
            aligned=(wakes[n][0]-.002<=begin<=end and begin<=callback+.002
                     and shown>=begin-.00001 and shown<=callback+.002)
            if aligned:
                latency=(shown-begin)*1000;end_latency=(shown-end)*1000
                phase=((begin-shown)*1000)%period
        samples.append(dict(frame=n,shown_s=shown,handover=handover,clock_aligned=aligned,
                            acquisition_ms=acquisition.get(n),present_to_display_ms=latency,
                            handover_end_to_display_ms=end_latency,observed_refresh_phase_ms=phase,
                            late=None if previous is None else (shown-previous)*1000>1.5*period))
        previous=shown
    groups={}
    for name,late in (('late',True),('on_time',False)):
        selected=[s for s in samples if s['late'] is late and s['clock_aligned']]
        groups[name]=dict(count=len(selected),present_to_display_ms=spread([s['present_to_display_ms'] for s in selected]),
                          observed_refresh_phase_ms=spread([s['observed_refresh_phase_ms'] for s in selected]))
    return dict(samples=samples,
                acquisition_ms=spread([v for n,v in acquisition.items() if n in wakes]),
                present_to_display_ms=spread([s['present_to_display_ms'] for s in samples if s['clock_aligned']]),
                handover_end_to_display_ms=spread([s['handover_end_to_display_ms'] for s in samples if s['clock_aligned']]),
                coverage=dict(shown_records=len(samples),handover_records=sum(s['handover'] is not None for s in samples),
                              missing_handover=sum(s['handover'] is None for s in samples),
                              clock_mismatch=sum(s['clock_aligned'] is False for s in samples)),
                phase_definition='handover start modulo the observed shown refresh grid, not an absolute link tick',groups=groups)


def native_floor(args, size, hz, scale, present, label):
    """Independent CADisplayLink/Metal window; no zgx pacing in this process."""
    out=args.output/label
    out.mkdir()
    mode='tiles' if present=='0' else 'drawables'
    width,height=SIZES[size]
    native_size=[width*scale/args.native_scale,height*scale/args.native_scale]
    binary=ROOT/'target/scroll-native'
    binary_hash=sha(binary)
    source_hash=sha(ROOT/'scripts/perf/scroll_native.swift')
    def invalid(message):
        result=dict(status='invalid',failures=[message],mode=mode,
                    binary_sha256=binary_hash,source_sha256=source_hash)
        (out/'run.json').write_text(json.dumps(result,indent=2)+'\n')
        return result
    try:
        with (out/'native.log').open('w') as log:
            subprocess.run([str(binary),*map(str,native_size),str(hz),mode,'10'],
                           stdout=log,stderr=log,check=True,timeout=30)
    except (subprocess.TimeoutExpired,subprocess.CalledProcessError) as error:
        return invalid(str(error))
    begin=end=None
    wakes={}; gpu=[]; work=[]; presents=[]; unavailable=0; handovers={}; acquisition={}
    for line in (out/'native.log').read_text().splitlines():
        parts=line.split()
        if not parts: continue
        if parts[0]=='NATIVE_BEGIN': begin=float(parts[1])
        elif parts[0]=='NATIVE_END': end=float(parts[1])
        elif parts[0]=='NATIVE_WAKE': wakes[int(parts[1])]=list(map(float,parts[2:]))
        elif parts[0]=='NATIVE_PRESENT': presents.append(list(map(float,parts[1:])))
        elif parts[0]=='NATIVE_GPU': gpu.append((int(parts[1]),float(parts[2])))
        elif parts[0]=='NATIVE_WORK': work.append((int(parts[1]),float(parts[2])))
        elif parts[0]=='NATIVE_HANDOVER': handovers[int(parts[1])]=list(map(float,parts[2:]))
        elif parts[0]=='NATIVE_ACQUIRE': acquisition[int(parts[1])]=float(parts[2])
        elif parts[0]=='NATIVE_UNAVAILABLE': unavailable+=1
    if begin is None or end is None: return invalid('native floor missing sample anchors')
    sample={n:v for n,v in wakes.items() if begin<=v[0]<=end}
    presented=sorted(p[2] for p in presents if int(p[0]) in sample and p[2]>0)
    gaps=[1000*(b-a) for a,b in zip(presented,presented[1:])]
    skipped=sum(p[2]<=0 for p in presents if int(p[0]) in sample)
    late=sum(gap>1.5*1000/hz for gap in gaps)
    m=dict(mode=mode,native_points=native_size,requested_scale=scale,hz=hz,
           presentation_timings=native_presentation_timings(presents,sample,handovers,acquisition,1000/hz),
           late_frames=late,skipped_drawables=skipped,late_pct=100*late/max(1,len(gaps)),
           interval_ms=spread(gaps),main_work_ms=spread([v for n,v in work if n in sample]),
           gpu_p99_ms=percentile([v for n,v in gpu if n in sample],.99),
           wake_after_timestamp_ms=spread([1000*(v[0]-v[1]) for v in sample.values()]),
           wake_after_target_ms=spread([1000*(v[0]-v[2]) for v in sample.values()]),
           display_period_ms=spread([1000*(v[2]-v[1]) for v in sample.values()]),
           samples=len(presented),unavailable_drawables=unavailable,
           status='valid',failures=[],binary_sha256=binary_hash,source_sha256=source_hash)
    if len(presented)<hz*8: return invalid('native floor lacks sustained feedback')
    if abs(m['display_period_ms']['p50']-1000/hz)>.5:
        return invalid('native display clock differs from requested refresh')
    if m['gpu_p99_ms'] is None or m['main_work_ms']['max'] is None:
        return invalid('native floor lacks work/GPU evidence')
    (out/'run.json').write_text(json.dumps(m,indent=2)+'\n')
    return m


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('command', choices=['run', 'headless', 'reference'])
    p.add_argument('--runner', type=Path, default=ROOT / 'target/dist/zgx-run')
    p.add_argument('--engine-source',type=Path,default=ROOT,help='engine checkout that produced the binaries')
    p.add_argument('--engine-pin',help='canonical engine commit under comparison, before suite instrumentation')
    p.add_argument('--zeron-source',type=Path,default=Path.home()/'Documents/GitHub/zeron-scrollbench')
    p.add_argument('--zeron-runner',type=Path,default=ROOT/'target/zeron-native/current/zeron-zgx')
    p.add_argument('--reference-runner', type=Path, default=ROOT / 'target/dist/examples/scroll_reference')
    p.add_argument('--output', type=Path, default=ROOT / 'target/scroll-results' / time.strftime('%Y%m%d-%H%M%S'))
    p.add_argument('--scenes', default='transcript,plain,grid,nested')
    p.add_argument('--gestures', default='slow,flick,reversals,notches,fling,wing,stream')
    p.add_argument('--sizes', default='S,M,L')
    p.add_argument('--hz', default='120')
    p.add_argument('--host-env',action='append',default=[],help='diagnostic ZGX_VIEW_CLOCK=0|1 or ZGX_SCROLL_LINK=0|1; repeat for both')
    p.add_argument('--require-frame-source',action='store_true',help='invalidate a pacing arm without its observed source-mode note')
    for name in ('reference-source','scroll-source'):
        p.add_argument('--'+name,choices=['view link','display link','timer on the grid'],help='expected mode for an engine with a changed default; verified from frame notes')
    p.add_argument('--reference-refresh-grid',choices=['off','on'],default='off',help='on exercises latch holds in explicit pacing probes')
    p.add_argument('--native-floor',action='store_true',help='bracket each zgx idle reference with independent Swift/Metal references')
    p.add_argument('--set-refresh', action='store_true', help='select matching native display modes and restore the original')
    p.add_argument('--scale', default='2')
    p.add_argument('--reset-row',type=int,default=160,help='real-app message row to align before each run;0 retains legacy fractional reset')
    p.add_argument('--native-scale', type=float, default=2, help='display backing scale; requested render scales use app zoom')
    p.add_argument('--backgrounds', default='opaque,translucent')
    p.add_argument('--present', default='0,1', help='comma-separated 0 (tiles), 1 (drawables), auto')
    p.add_argument('--repeats', type=int, default=3)
    p.add_argument('--warmup', type=float, default=3)
    p.add_argument('--rest-seconds', type=float, default=0, help='sample RSS after an input-free settling period; no inspector polling during the wait')
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
    if not math.isfinite(args.rest_seconds) or not 0 <= args.rest_seconds <= 60:
        p.error('--rest-seconds must be finite and between 0 and 60')
    for item in args.host_env:
        if item not in ('ZGX_VIEW_CLOCK=0','ZGX_VIEW_CLOCK=1','ZGX_SCROLL_LINK=0','ZGX_SCROLL_LINK=1','ZGX_SCROLL_DRAWABLES=0','ZGX_SCROLL_DRAWABLES=1','ZGX_HANDOVER_ASAP=0','ZGX_HANDOVER_ASAP=1','ZGX_LEAD_US=4000','ZGX_LEAD_US=5500','ZGX_GPU_STATS=1','ZGX_SHADOW_WHOLE=1','ZGX_DRAWABLE_PREFETCH=0','ZGX_DRAWABLE_PREFETCH=1'):
            p.error('unsupported diagnostic host environment override')
    if args.command=='reference' and any(item.startswith('ZGX_LEAD_US=') for item in args.host_env) and args.reference_refresh_grid!='on':
        p.error('LEAD_US marker probes require --reference-refresh-grid on to exercise the hold')
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
    args.engine_source=args.engine_source.resolve()
    args.engine_commit=subprocess.check_output(['git','rev-parse','HEAD'],cwd=args.engine_source,text=True).strip()
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
    native_proof=args.zeron_runner.parent/'build.json'
    if native_proof.is_file():
        proof=json.loads(native_proof.read_text())
        if proof['binary_sha256']!=sha(args.zeron_runner):
            p.error('actual-app binary differs from its build proof')
        audit['native_build_proof']=proof
        audit['live_engine_source_commit']=args.engine_commit
        args.engine_commit=proof['zgx_commit']
    audit['engine_commit']=args.engine_commit
    audit['engine_source_diff']=subprocess.check_output(['git','diff','HEAD'],cwd=args.engine_source,text=True)
    if args.engine_pin:
        audit['engine_delta_from_pin']=subprocess.check_output(['git','diff',args.engine_pin,args.engine_commit],cwd=args.engine_source,text=True)
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
                    if any(ref[k].get('status')=='invalid' for k in ('native_floor_before','native_floor_after')):
                        ref['failures'].append('independent native reference invalid')
                        ref['status']='fail'
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
