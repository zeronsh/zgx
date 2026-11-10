#!/usr/bin/env python3
"""Binary sizes of what UI apps ship, base against head: nothing may move.

    python3 scripts/perf/sizes.py BASE [--markdown out.md] [--keep]

Builds this checkout and BASE, each with `cargo build --release --locked`:
- the gate's UI scenes (`gate.SCENES`, less zgx-3d's and zgx-video's
  benches);
- `zgx-run` with its default features less `scene3d` and `gltf` (what a UI
  app links).

BASE is checked out in a worktree at a path as long as this checkout's
(`base_dir`), since a binary keeps its sources' paths and their length moves
its size. It is built into a target directory of its own.

For each binary, it compares:
- **stripped size:** a copy run through `strip`;
- **symbols:** each defined function's or datum's size, read from the
  unstripped binary as the distance to the next symbol in address order
  (`llvm-nm -n`, which reads ELF and Mach-O alike; the copy rustup's
  llvm-tools installs is found in rustc's sysroot). Names are demangled and
  rustc's hashes (`::h0123456789abcdef`) and LLVM's `.llvm.N` suffixes are
  dropped, so a symbol keeps its name across builds.

Prints each binary's stripped size and the symbols that moved (largest
first; added and removed included). Exits 1 if any binary's stripped size or
any symbol's size moved, 0 if nothing did. The 3D design's promise that UI
apps pay nothing for 3D work is this script passing on each 3D change.
"""
import argparse
import collections
import hashlib
import os
import pathlib
import re
import shutil
import subprocess
import sys

sys.path.insert(0, str(pathlib.Path(__file__).parent))
import gate  # noqa: E402

ROOT = gate.ROOT
WORK = gate.WORK
# Not UI apps' code: 3D and video benches link their own renderers.
NOT_UI = ('zgx-3d', 'zgx-video')
# zgx-run's defaults that bring in 3D.
RUN_WITHOUT = ('scene3d', 'gltf')

HASH = re.compile(r'::h[0-9a-f]{16}$')
LLVM_SUFFIX = re.compile(r'\.llvm\.\d+$')


def base_dir():
    """A directory for the base exactly as long as this checkout's path
    (as gate.py's `base_checkout`, under a name of sizes' own)."""
    want = len(str(ROOT.resolve()))
    tag = hashlib.sha1(str(ROOT.resolve()).encode()).hexdigest()
    for parent in (WORK, pathlib.Path('/tmp')):
        room = want - len(str(parent.resolve())) - 1
        if room >= 10:
            return parent / (f'sizes-base-{tag}' + '_' * room)[:room]
    sys.exit(f'no directory for the base as short as the checkout ({ROOT})')


def ui_scenes(checkout):
    """(package, example) of the gate's UI scenes this checkout has."""
    seen = []
    for _, package, example, _ in gate.SCENES:
        if package in NOT_UI or (package, example) in seen:
            continue
        if gate.has_example(checkout, package, example):
            seen.append((package, example))
    return seen


def run_features(checkout):
    """zgx-run's default features less the 3D ones."""
    manifest = (checkout / 'crates' / gate.package_in(checkout, 'zgx-run') / 'Cargo.toml').read_text()
    found = re.search(r'^default\s*=\s*\[([^\]]*)\]', manifest, re.M)
    defaults = re.findall(r'"([^"]+)"', found.group(1)) if found else []
    return [f for f in defaults if f not in RUN_WITHOUT]


def build(checkout, target, into):
    """Builds the UI binaries of `checkout` into `target` and copies them
    to `into`; their names."""
    env = dict(os.environ, CARGO_TARGET_DIR=str(target), CARGO_INCREMENTAL='0',
               CARGO_PROFILE_RELEASE_DEBUG='0')
    scenes = ui_scenes(checkout)
    # As gate.py builds them: a package of ALONE's on its own, so its
    # dev-dependencies' features don't reach the others.
    parts = [[s for s in scenes if s[0] not in gate.ALONE]]
    parts += [[s for s in scenes if s[0] == alone] for alone in gate.ALONE]
    for part in parts:
        if not part:
            continue
        command = ['cargo', 'build', '--release', '--locked', '--quiet']
        for package in dict.fromkeys(p for p, _ in part):
            command += ['-p', gate.package_in(checkout, package)]
            for _, example in (s for s in part if s[0] == package):
                command += ['--example', example]
        subprocess.run(command, cwd=checkout, env=env, check=True)
    run = gate.package_in(checkout, 'zgx-run')
    subprocess.run(['cargo', 'build', '--release', '--locked', '--quiet', '-p', run, '--bin', run,
                    '--no-default-features', '--features', ','.join(run_features(checkout))],
                   cwd=checkout, env=env, check=True)
    into.mkdir(parents=True, exist_ok=True)
    names = []
    for _, example in scenes:
        shutil.copy2(target / 'release' / 'examples' / example, into / example)
        names.append(example)
    shutil.copy2(target / 'release' / run, into / 'zgx-run')
    names.append('zgx-run')
    return names


def llvm_nm():
    found = shutil.which('llvm-nm')
    if found:
        return found
    sysroot = subprocess.run(['rustc', '--print', 'sysroot'], capture_output=True, text=True,
                             check=True).stdout.strip()
    for candidate in pathlib.Path(sysroot, 'lib', 'rustlib').glob('*/bin/llvm-nm'):
        return str(candidate)
    sys.exit('no llvm-nm: `rustup component add llvm-tools`')


def normalize(name):
    return LLVM_SUFFIX.sub('', HASH.sub('', name))


def symbol_sizes(nm_output):
    """Sizes by normalized name from `llvm-nm -n --defined-only -C` output:
    each symbol's distance to the next, in its own section kind (text or
    data); the last of a kind, whose end is unknown, is left out. Of names
    at one address (aliases), the last by name holds the size. Names that
    normalize alike add up."""
    rows = []
    for line in nm_output.splitlines():
        parts = line.split(None, 2)
        if len(parts) != 3:
            continue
        address, kind, name = parts
        try:
            rows.append((int(address, 16), kind.lower(), name))
        except ValueError:
            continue
    sizes = collections.Counter()
    by_kind = collections.defaultdict(list)
    for row in rows:
        by_kind[row[1] in 'tw'].append(row)
    for kind_rows in by_kind.values():
        kind_rows.sort()
        for (address, _, name), (following, _, _) in zip(kind_rows, kind_rows[1:]):
            if following > address:
                sizes[normalize(name)] += following - address
    return sizes


def stripped_size(binary, scratch):
    copy = scratch / (binary.name + '.stripped')
    shutil.copy2(binary, copy)
    subprocess.run(['strip', str(copy)], check=True, capture_output=True)
    size = copy.stat().st_size
    copy.unlink()
    return size


def measure(binary, scratch, nm):
    out = subprocess.run([nm, '-n', '--defined-only', '-C', str(binary)], capture_output=True, text=True,
                         check=True).stdout
    return stripped_size(binary, scratch), symbol_sizes(out)


def moved(before, after):
    """Symbols whose size differs, added or removed: (name, base, head)."""
    rows = []
    for name in set(before) | set(after):
        b, h = before.get(name), after.get(name)
        if b != h:
            rows.append((name, b, h))
    rows.sort(key=lambda r: -abs((r[2] or 0) - (r[1] or 0)))
    return rows


def report(name, size_before, size_after, rows, shown=15):
    lines = [f'{name}: stripped {size_before:,} -> {size_after:,} bytes ({size_after - size_before:+,})']
    for symbol, b, h in rows[:shown]:
        change = (h or 0) - (b or 0)
        state = 'added' if b is None else 'removed' if h is None else f'{b:,} -> {h:,}'
        lines.append(f'  {change:+9,}  {state:>22}  {symbol}')
    if len(rows) > shown:
        lines.append(f'  ... and {len(rows) - shown} more symbols')
    return lines


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('base')
    parser.add_argument('--markdown', type=pathlib.Path)
    parser.add_argument('--keep', action='store_true', help="keep the base's worktree")
    args = parser.parse_args()
    nm = llvm_nm()
    base = base_dir()
    if base.exists():
        subprocess.run(['git', 'worktree', 'remove', '--force', str(base)], cwd=ROOT, capture_output=True)
        shutil.rmtree(base, ignore_errors=True)
    subprocess.run(['git', 'worktree', 'add', '--detach', str(base), args.base], cwd=ROOT, check=True,
                   capture_output=True)
    scratch = WORK / 'sizes'
    scratch.mkdir(parents=True, exist_ok=True)
    try:
        head_names = build(ROOT, WORK / 'sizes-target-head', scratch / 'head')
        base_names = build(base, WORK / 'sizes-target-base', scratch / 'base')
    finally:
        if not args.keep:
            subprocess.run(['git', 'worktree', 'remove', '--force', str(base)], cwd=ROOT, capture_output=True)
    lines, changed = [], []
    for name in head_names:
        if name not in base_names:
            lines.append(f'{name}: new (no base)')
            continue
        size_b, syms_b = measure(scratch / 'base' / name, scratch, nm)
        size_h, syms_h = measure(scratch / 'head' / name, scratch, nm)
        rows = moved(syms_b, syms_h)
        if size_b != size_h or rows:
            changed.append(name)
        lines += report(name, size_b, size_h, rows)
    print('\n'.join(lines))
    verdict = (f'SIZES MOVED: {", ".join(changed)}' if changed
               else f'SIZES UNCHANGED: {len(head_names)} binaries, every symbol the same size')
    print(verdict)
    if args.markdown:
        args.markdown.write_text('```\n' + '\n'.join(lines) + '\n```\n\n' + verdict + '\n')
    sys.exit(1 if changed else 0)


if __name__ == '__main__':
    main()
