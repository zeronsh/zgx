#!/usr/bin/env python3
"""The instruction gate's table read as numbers, against the bar the 3D and
audio work is held to: every UI cell within +0.02%, each 3D cell within its
own no-change spread (`gate.SPREAD`, or 0.1% where none is recorded).

    python3 scripts/perf/read_gate.py perf.md

gate.py fails only above +1%, so its verdict says little here. This prints
each row's change as a number with the bar it was read against, and exits
1 if any cell went past its bar. New cells (no base) are listed, not read.
"""
import pathlib
import re
import sys

sys.path.insert(0, str(pathlib.Path(__file__).parent))
import gate  # noqa: E402

UI_BAR = 0.02
UNKNOWN_3D_BAR = 0.1
ROW = re.compile(r'^\|\s*([\w.-]+)\s*\|\s*([\d,.]+) M\s*\|\s*([\d,.]+) M\s*\|\s*([+-][\d.]+)%')
NEW = re.compile(r'^\|\s*([\w.-]+)\s*\|\s*\(new\)')


def bar(name):
    if name.startswith(('3d-', 'audio-')):
        return gate.SPREAD.get(name, UNKNOWN_3D_BAR)
    return UI_BAR


def read(text):
    """(name, base, head, change %, bar %, over) for each row; new cells."""
    rows, new = [], []
    for line in text.splitlines():
        found = ROW.match(line)
        if found:
            name = found.group(1)
            base, head = (float(found.group(i).replace(',', '')) for i in (2, 3))
            change = float(found.group(4))
            rows.append((name, base, head, change, bar(name), change > bar(name)))
            continue
        found = NEW.match(line)
        if found:
            new.append(found.group(1))
    return rows, new


def main():
    rows, new = read(pathlib.Path(sys.argv[1]).read_text())
    if not rows:
        sys.exit('no gate rows found')
    for name, base, head, change, limit, over in rows:
        flag = '  OVER' if over else ''
        print(f'{name:28} {base:12,.1f} M {head:12,.1f} M {change:+8.3f}%  (bar +{limit:g}%){flag}')
    for name in new:
        print(f'{name:28} new: no base to read against')
    over = [r[0] for r in rows if r[5]]
    print(f'OVER THE BAR: {", ".join(over)}' if over else f'every cell within its bar ({len(rows)} read)')
    sys.exit(1 if over else 0)


if __name__ == '__main__':
    main()
