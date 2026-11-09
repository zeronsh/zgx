#!/usr/bin/env python3
"""Interleaved baseline/variant scroll trials; see docs/scroll-benchmark.md."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import statistics
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parents[2]


def stats(values):
    values = [v for v in values if v is not None]
    return dict(n=len(values), mean=statistics.mean(values) if values else None,
                min=min(values) if values else None, max=max(values) if values else None,
                stdev=statistics.stdev(values) if len(values)>1 else None, values=values)


def summarize(samples, trials):
    rows=[]
    for arm in dict.fromkeys(s['arm'] for s in samples):
        for role in ('base','variant'):
            selected=[s for s in samples if s['arm']==arm and s['role']==role]
            valid=[s for s in selected if s['valid']]
            metric=lambda key: stats([s['metrics'].get(key) for s in valid])
            rows.append(dict(arm=arm,role=role,n=len(valid),required=trials,
                             complete=len(valid)==trials,
                             condition=next(s['condition'] for s in samples if s['arm']==arm),
                             late_pct=metric('late_pct'),interval_p99_ms=stats([s['metrics']['interval_ms']['p99'] for s in valid]),
                             main_over_4ms=metric('main_over_4ms'),gpu_p99_ms=metric('gpu_p99_ms'),
                             shown_frames=metric('shown_frames_tape'),
                             same_refresh_extra_frames=metric('same_refresh_extra_frames'),
                             double_then_skip=metric('double_then_skip_intervals'),
                             main_max_ms=stats([s['metrics']['main_work_ms']['max'] for s in valid]),
                             repaint_pct=metric('repaint_pct'),
                             main_with_present_max_ms=stats([s['metrics'].get('main_with_present_ms',{}).get('max') for s in valid]),
                             rows_built=metric('rows_built_in_scroll_frames'),
                             picture_frames=metric('picture_frames'),
                             exact_latency_p99_ms=stats([s['metrics'].get('start_to_present_ms',{}).get('p99') for s in valid]),
                             estimated_latency_p99_ms=stats([s['metrics'].get('render_start_to_shown_ms_estimate',{}).get('p99') for s in valid]),
                             frame_end_to_shown_p99_ms=stats([s['metrics'].get('frame_end_to_shown_ms_estimate',{}).get('p99') for s in valid]),
                             rss_delta_kib=stats([s['metrics'].get('rss_kib',{}).get('delta') for s in valid]),
                             all_gates_pass=bool(valid) and len(valid)==trials and all(s['status']=='pass' for s in valid)))
    for row in rows:
        pairs=[]
        for trial in range(trials):
            selected=[s for s in samples if s['arm']==row['arm'] and s.get('trial')==trial and s['valid']]
            base=next((s for s in selected if s['role']=='base'),None)
            variant=next((s for s in selected if s['role']=='variant'),None)
            if base is None or variant is None:continue
            a,b=base['metrics'],variant['metrics']
            pairs.append(dict(trial=trial,late_delta_pp=b['late_pct']-a['late_pct'],
                              shown_delta=b.get('shown_frames_tape',0)-a.get('shown_frames_tape',0)))
        row['paired_trials']=pairs
        row['paired_late_delta_pp']=stats([p['late_delta_pp'] for p in pairs])
        row['paired_shown_delta']=stats([p['shown_delta'] for p in pairs])
        # A smoother cadence achieved by showing fewer frames loses the experiment.
        row['shown_count_regression']=bool(pairs) and row['paired_shown_delta']['mean']<0
        if row['role']=='variant' and row['shown_count_regression']:
            row['all_gates_pass']=False
    return rows


def table(rows):
    lines=['| Arm | Role | Scene / gesture / size / present | Trials | Late mean [min,max]; SD % | Interval p99 mean ms | Main >4ms mean [min,max] | Shown mean [min,max] | Paired shown Δ | Gates |',
           '|---|---|---|---:|---|---:|---|---|---:|---|']
    def number(v):return '—' if v is None else f'{v:.2f}'
    def spread(s):return f"{number(s['mean'])} [{number(s['min'])},{number(s['max'])}]"
    for r in rows:
        c=r['condition']
        lines.append(f"| {r['arm']} | {r['role']} | {c.get('scene','zeron')} / {c.get('gesture','wing')} / {c.get('size','W')} / {c.get('present','0')} | {r['n']}/{r['required']} | {spread(r['late_pct'])}; {number(r['late_pct']['stdev'])} | {number(r['interval_p99_ms']['mean'])} | {spread(r['main_over_4ms'])} | {spread(r['shown_frames'])} | {number(r['paired_shown_delta']['mean'])} | {'pass' if r['all_gates_pass'] else 'fail' if r['complete'] else 'incomplete'} |")
    return '\n'.join(lines)+'\n'


def anchor(result):
    value=result.get('setup_anchor',{}).get('chat.transcript.anchor')
    return value if isinstance(value,dict) else None


def run_one(args, descriptor, arm, role, trial, position):
    c=arm.get('condition',{})
    label=f"t{trial+1:02}-{position+1:02}-{arm['name']}-{role}"
    output=args.output/'runs'/label
    result_file=output/'comparison-sample.json'
    if result_file.exists():return json.loads(result_file.read_text())
    command=c.get('command','run')
    argv=[sys.executable,str(ROOT/'scripts/perf/scroll.py'),command,
          '--scenes',c.get('scene','zeron'),'--gestures',c.get('gesture','wing'),
          '--sizes',c.get('size','W'),'--hz',str(c.get('hz',120)),
          '--scale',str(c.get('scale',2)),'--backgrounds',c.get('background','translucent'),
          '--present',str(c.get('present','0')),'--repeats','1','--native-floor',
          '--reset-row',str(c.get('reset_row',160)), '--output',str(output)]
    for key in ('engine_source','engine_pin','zeron_source','zeron_runner','runner','reference_runner'):
        if key in descriptor:argv+=['--'+key.replace('_','-'),str(descriptor[key])]
    for value in descriptor.get('env',[]):argv+=['--host-env',value]
    console=args.output/'console'/f'{label}.log'
    print('START',label,time.strftime('%H:%M:%S',time.gmtime()),flush=True)
    with console.open('w') as log:completed=subprocess.run(argv,cwd=ROOT,stdout=log,stderr=subprocess.STDOUT)
    wanted='zeron-idle' if command=='reference' and c.get('scene','zeron')=='zeron' else ('idle' if command=='reference' else c.get('scene','zeron'))
    data=json.loads((output/'summary.json').read_text()) if (output/'summary.json').exists() else []
    found=[r for r in data if r['scene']==wanted]
    r=found[-1] if found else dict(status='invalid',failures=['missing requested result'])
    m=r.get('metrics',{})
    valid=r['status']!='invalid' and m.get('active_intervals',0)>=30 and m.get('gpu_p99_ms') is not None
    refs=[r for r in data if r['scene'] in ('idle','zeron-idle')]
    native_valid=bool(refs) and all(ref.get(k,{}).get('status')=='valid' for ref in refs for k in ('native_floor_before','native_floor_after'))
    sample=dict(arm=arm['name'],role=role,trial=trial,condition=c,descriptor=descriptor,
                valid=valid and native_valid,status=r['status'],metrics=m,
                failures=r.get('failures',[]),setup_anchor=r.get('setup_anchor',{}),
                references=refs,evidence=str(output),exit_code=completed.returncode)
    output.mkdir(parents=True,exist_ok=True)
    result_file.write_text(json.dumps(sample,indent=2)+'\n')
    print('END',label,'valid',sample['valid'],'late',m.get('late_pct'),flush=True)
    return sample


def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--manifest',type=Path,required=True)
    p.add_argument('--output',type=Path,required=True)
    p.add_argument('--trials',type=int,default=3)
    args=p.parse_args()
    if args.trials<3:p.error('comparisons require at least three interleaved trials')
    manifest=json.loads(args.manifest.read_text())
    if not manifest.get('arms'):p.error('manifest must contain at least one arm')
    names=[arm['name'] for arm in manifest['arms']]
    if len(set(names))!=len(names):p.error('arm names must be unique')
    args.output.mkdir(parents=True,exist_ok=True)
    (args.output/'runs').mkdir(exist_ok=True);(args.output/'console').mkdir(exist_ok=True)
    frozen=args.output/'manifest.json'
    if frozen.exists() and json.loads(frozen.read_text())!=manifest:p.error('resume manifest differs from saved manifest')
    frozen.write_text(json.dumps(manifest,indent=2)+'\n')
    fingerprint=hashlib.sha256()
    for name in ('scroll.py','scroll_compare.py','scroll_native.swift','zeron_fixture.luau'):
        fingerprint.update((ROOT/'scripts/perf'/name).read_bytes())
    saved=args.output/'harness-sha256.txt'
    if saved.exists() and saved.read_text().strip()!=fingerprint.hexdigest():
        p.error('harness changed; start a new comparison batch')
    saved.write_text(fingerprint.hexdigest()+'\n')
    assertion=subprocess.Popen(['caffeinate','-d','-i','-w',str(os.getpid())]) if sys.platform=='darwin' else None
    samples=[]
    try:
        arms=manifest['arms']
        for trial in range(args.trials):
            shift=trial%len(arms)
            for position,arm in enumerate(arms[shift:]+arms[:shift]):
                pair=[]
                for role,descriptor in [('base',manifest['baseline']),('variant',arm)]:
                    sample=run_one(args,descriptor,arm,role,trial,position)
                    pair.append(sample);samples.append(sample)
                if arm.get('condition',{}).get('scene','zeron')=='zeron':
                    a,b=[anchor(s) for s in pair]
                    if a is None or b is None or a.get('id')!=b.get('id') or abs(a.get('dy',0)-b.get('dy',0))>.05:
                        for s in pair:s['valid']=False;s['failures'].append('paired transcript anchors differ or are missing')
                rows=summarize(samples,args.trials)
                (args.output/'samples.json').write_text(json.dumps(samples,indent=2)+'\n')
                (args.output/'summary.json').write_text(json.dumps(rows,indent=2)+'\n')
                (args.output/'summary.md').write_text(table(rows))
                print(table(rows),flush=True)
    finally:
        if assertion is not None:assertion.terminate();assertion.wait()
    return int(any(not r['all_gates_pass'] for r in summarize(samples,args.trials)))


if __name__=='__main__':sys.exit(main())
