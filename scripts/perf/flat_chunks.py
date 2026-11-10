#!/usr/bin/env python3
"""Whether one chunk of a streamed Markdown reply costs the same however
long the reply has grown: `md_chunk_bench` counted whole under callgrind at
100, 200, 400 and 800 sections, once with 9 chunks and once with 27, a
chunk's cost the difference over 18 (what fills the reply and tears the
window down is in both, so it drops out; `--toggle-collect` on the chunks'
function went on counting the teardown).

The verdict is on zgx's own instructions: the program's, less the C
library's (glibc's malloc and free) and Luau's memory manager and collector
(allocation, free, sweep and mark). Those move from run to run with the
heap's layout and the collector's pace: one tree read 1.11x and then 0.96x
whole, with zgx's own functions identical in every run. Exits 1 when a
chunk at the longest reply costs more than FLAT times one at the shortest
(`--report` only prints). Linux, with valgrind (CI's `perf` job);
`tests/md_chunk_flat.rs` splits a chunk by phase.

    python3 scripts/perf/flat_chunks.py [--report] [--markdown FILE]
"""
import argparse
import collections
import os
import pathlib
import re
import subprocess
import sys
import tempfile

SIZES = [100, 200, 400, 800]
CHUNKS = (9, 27)
FLAT = 1.25
ROOT = pathlib.Path(__file__).resolve().parents[2]
BENCH = 'md_chunk_bench'
# Luau's memory manager and collector, by function (it is linked into the
# program), and mimalloc's, were it the program's allocator.
NOT_OWN = re.compile(
    r'^(luaC_|luaM_|luaH_free|luaF_free|luaU_freeudata|luaS_free|luaE_free|sweep|propagatemark|'
    r'reallymarkobject|markudata|markroot|deletegco|freeblock|newblock|newpage|freepage|gcstep|'
    r'singlestep|atomic\(|clearkeys|clearvalues|shrinkbuffers|mi_|_mi_)')


def build(target):
    subprocess.run(
        ['cargo', 'build', '--release', '--locked', '-q', '-p', 'zgx-script', '--example', BENCH],
        cwd=ROOT, check=True, env={**os.environ, 'CARGO_TARGET_DIR': str(target)})
    return target / 'release' / 'examples' / BENCH


def counts(out):
    """The whole program's instructions and zgx's own, from a callgrind
    output: each function's own cost (`fn=` then its cost lines, a call's
    inclusive cost, the line after `calls=`, skipped), by object."""
    names = collections.defaultdict(dict)
    kinds = {'ob': 'ob', 'cob': 'ob', 'fn': 'fn', 'cfn': 'fn'}
    ob = fn = ''
    whole = own = 0
    mine = False
    after_call = False
    for line in out.read_text().splitlines():
        if after_call:
            after_call = False
            continue
        if not line:
            continue
        if line[0].isdigit() or line[0] in '+-*':
            parts = line.split()
            if len(parts) > 1:
                cost = int(parts[1])
                whole += cost
                if mine:
                    own += cost
            continue
        key, _, rest = line.partition('=')
        if key in kinds:
            # `(id) name` names an id; `(id)` alone uses it again.
            table = names[kinds[key]]
            match = re.match(r'\((\d+)\)\s*(.*)', rest)
            name = rest
            if match:
                name = table.setdefault(match[1], match[2]) if match[2] else table.get(match[1], '')
            if key == 'ob':
                ob = name
            elif key == 'fn':
                fn = re.sub(r"'\d+$", '', name)
            if key in ('ob', 'fn'):
                mine = ob.endswith('/' + BENCH) and not NOT_OWN.match(fn)
        elif key == 'calls':
            after_call = True
    if whole == 0:
        raise SystemExit(f'no costs in {out}')
    return whole, own


def total(binary, sections, chunks, scratch):
    out = scratch / f'md-chunk-{sections}-{chunks}.out'
    env = {**os.environ, 'SECTIONS': str(sections), 'CHUNKS': str(chunks), 'ZGX_GATE': '1', 'ZGX_ASSISTIVE': '0'}
    command = ['valgrind', '--tool=callgrind', f'--callgrind-out-file={out}', str(binary)]
    if subprocess.run(['which', 'setarch'], capture_output=True).returncode == 0:
        command = ['setarch', '-R', *command]
    subprocess.run(command, env=env, check=True, capture_output=True)
    return counts(out)


def per_chunk(binary, sections, scratch):
    """A chunk's (whole, own) instructions at `sections`."""
    few, many = CHUNKS
    a, b = total(binary, sections, few, scratch), total(binary, sections, many, scratch)
    return tuple((y - x) // (many - few) for x, y in zip(a, b))


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--report', action='store_true', help='print, never fail')
    parser.add_argument('--markdown', type=pathlib.Path, help='append the table here too')
    args = parser.parse_args()
    target = pathlib.Path(os.environ.get('ZGX_GATE_TARGET', ROOT / 'target' / 'flat-chunks'))
    binary = build(target)
    with tempfile.TemporaryDirectory() as scratch:
        chunks = [per_chunk(binary, s, pathlib.Path(scratch)) for s in SIZES]
    whole, own = [c[0] for c in chunks], [c[1] for c in chunks]
    growth = own[-1] / own[0]
    lines = ['| sections | ' + ' | '.join(str(s) for s in SIZES) + ' |',
             '|---|' + '---|' * len(SIZES),
             "| zgx's own instructions per chunk | " + ' | '.join(f'{c:,}' for c in own) + ' |',
             "| the whole program's (with malloc and Luau's collector) | "
             + ' | '.join(f'{c:,}' for c in whole) + ' |',
             '',
             f"A chunk at {SIZES[-1]} sections costs {growth:.2f}x one at {SIZES[0]} in zgx's own "
             f"instructions (flat: at most {FLAT}x); the whole program's, {whole[-1] / whole[0]:.2f}x."]
    text = '\n'.join(lines)
    print(text)
    if args.markdown:
        # Appended: a job's summary holds other steps' too.
        with args.markdown.open('a') as summary:
            summary.write('### Streamed Markdown, per chunk\n\n' + text + '\n')
    if growth > FLAT and not args.report:
        sys.exit(1)


if __name__ == '__main__':
    main()
