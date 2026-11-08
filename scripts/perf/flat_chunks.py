#!/usr/bin/env python3
"""Whether one chunk of a streamed Markdown reply costs the same however
long the reply has grown: `md_chunk_bench` counted whole under callgrind at
100, 200, 400 and 800 sections, once with 9 chunks and once with 27, a
chunk's cost the difference over 18 (what fills the reply and tears the
window down is in both, so it drops out; `--toggle-collect` on the chunks'
function went on counting the teardown). Exits 1 when a chunk at the longest reply costs more than FLAT times
one at the shortest (`--report` only prints). Linux, with valgrind (CI's
`perf` job); `tests/md_chunk_flat.rs` splits a chunk by phase.

    python3 scripts/perf/flat_chunks.py [--report] [--markdown FILE]
"""
import argparse
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


def build(target):
    subprocess.run(
        ['cargo', 'build', '--release', '--locked', '-q', '-p', 'zgx-script', '--example', 'md_chunk_bench'],
        cwd=ROOT, check=True, env={**os.environ, 'CARGO_TARGET_DIR': str(target)})
    return target / 'release' / 'examples' / 'md_chunk_bench'


def total(binary, sections, chunks, scratch):
    out = scratch / f'md-chunk-{sections}-{chunks}.out'
    env = {**os.environ, 'SECTIONS': str(sections), 'CHUNKS': str(chunks), 'ZGX_GATE': '1', 'ZGX_ASSISTIVE': '0'}
    command = ['valgrind', '--tool=callgrind', f'--callgrind-out-file={out}', str(binary)]
    if subprocess.run(['which', 'setarch'], capture_output=True).returncode == 0:
        command = ['setarch', '-R', *command]
    subprocess.run(command, env=env, check=True, capture_output=True)
    for line in out.read_text().splitlines():
        if line.startswith('totals:') or line.startswith('summary:'):
            return int(line.split()[1])
    raise SystemExit(f'no totals in {out}')


def per_chunk(binary, sections, scratch):
    few, many = CHUNKS
    return (total(binary, sections, many, scratch) - total(binary, sections, few, scratch)) // (many - few)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--report', action='store_true', help='print, never fail')
    parser.add_argument('--markdown', type=pathlib.Path, help='append the table here too')
    args = parser.parse_args()
    target = pathlib.Path(os.environ.get('ZGX_GATE_TARGET', ROOT / 'target' / 'flat-chunks'))
    binary = build(target)
    with tempfile.TemporaryDirectory() as scratch:
        counts = [per_chunk(binary, s, pathlib.Path(scratch)) for s in SIZES]
    growth = counts[-1] / counts[0]
    lines = ['| sections | ' + ' | '.join(str(s) for s in SIZES) + ' |',
             '|---|' + '---|' * len(SIZES),
             '| instructions per chunk | ' + ' | '.join(f'{c:,}' for c in counts) + ' |',
             '',
             f'A chunk at {SIZES[-1]} sections costs {growth:.2f}x one at {SIZES[0]} (flat: at most {FLAT}x).']
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
