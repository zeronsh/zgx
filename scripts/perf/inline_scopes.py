#!/usr/bin/env python3
"""Whether a hot function still inlines what it did: rustc's MIR inliner
stops at a depth and a budget, so a change far from a hot function can
make it stop inlining its own callees, with no change in its source
(`Scene::layer_items_walk` in #1740, `encode_draws` in #1824). The gate
shows the cost; this names the function before it.

Each crate's release MIR (`--emit=mir`, one build of the crate) is read
for its functions' inlined scopes (`scope N (inlined ...)`) and size (the
lines of its body), and the hot functions listed in `inline_scopes.txt`
are compared with what is recorded there. Either change can cost what the
gate measures: fewer scopes are callees no longer inlined (#1740); a body
grown by much has pulled callees in, and LLVM may then fold it into its
hot caller (`batches` into `encode_draws`, #1824). A function past either
line fails the check; with `--base REF`, the tree at REF is built too,
for that crate alone and only then, to name the callees it gained or lost.
Smaller changes are printed, to be recorded. Recording (`record`) writes
the file again from the tree as it is: a change of inlining is then a line
in the pull request.

The counts are a compiler's: the file names the rustc that recorded them,
and under another (a new stable in CI) the check prints the list that rustc
would record and passes, to be recorded with it.

    python3 scripts/perf/inline_scopes.py [check|record] [--root DIR] [--base REF]
"""
import argparse
import collections
import os
import pathlib
import re
import subprocess
import sys

HERE = pathlib.Path(__file__).resolve().parent
LIST = HERE / "inline_scopes.txt"
CRATES = ("zgx", "zgx-gpu", "zgx-script")
# Scopes a function may lose before it fails: a change of a callee's own
# body can add or drop one. A lost scope of note is a whole callee's inlining.
SLACK = 2
# How much a body may grow before it fails: by both a share and a count of
# lines, so a small function adding a few doesn't.
GROWTH = 1.25
GROWTH_LINES = 30


TRIPLE = None


def mir(root, crate, target):
    """`crate`'s release MIR, built under `target` (for `TRIPLE`, if set:
    MIR inlining can differ by target, and CI's is x86_64)."""
    # As the gate builds (gate.py's `build`): its release dependencies serve.
    env = dict(os.environ, CARGO_TARGET_DIR=str(target), CARGO_INCREMENTAL="0",
               CARGO_PROFILE_RELEASE_DEBUG="0")
    triple = ["--target", TRIPLE] if TRIPLE else []
    subprocess.run(["cargo", "rustc", "-q", "-p", crate, "--release", "--lib", "--locked", *triple,
                    "--", "--emit=mir"], cwd=root, env=env, check=True)
    name = crate.replace("-", "_")
    out = target / TRIPLE if TRIPLE else target
    dumps = sorted((out / "release" / "deps").glob(f"{name}-*.mir"), key=lambda p: p.stat().st_mtime)
    if not dumps:
        raise SystemExit(f"no MIR for {crate} under {target}")
    return dumps[-1].read_text(errors="replace")


def normal(name):
    """A function's name as MIR prints it, without where its `impl` block
    starts: an edit above a function doesn't rename it."""
    return re.sub(r"<impl at ([^>:]+):\d+:\d+: \d+:\d+>", r"<impl in \1>", name)


def scopes(text):
    """Each function's inlined callees by name, and its body's lines."""
    found = collections.defaultdict(list)
    lines = collections.Counter()
    current = None
    for line in text.splitlines():
        if current is not None:
            lines[current] += 1
        head = re.match(r"^fn (.+?)\(", line)
        if head:
            # Two functions one name (after `normal`): the second is `#2`.
            current, seen = normal(head.group(1)), 1
            while current in found:
                seen += 1
                current = f"{normal(head.group(1))}#{seen}"
            found[current]
            continue
        inlined = current and re.match(r"\s+scope \d+ \(inlined (.+)\) \{", line)
        if inlined:
            found[current].append(normal(inlined.group(1)))
    return found, lines


def rustc(root):
    """The compiler cargo runs in `root`, as `rustc -V` names it."""
    return subprocess.run(["rustc", "-V"], cwd=root, check=True, capture_output=True,
                          text=True).stdout.strip()


def recorded_by():
    """The rustc the file's counts are from (its `# rustc ...` line)."""
    for line in LIST.read_text().splitlines():
        if line.startswith("# rustc "):
            return line[2:]
    return None


def table(rows, got, compiler):
    """The file as `got` and `compiler` would record it."""
    header = [line for line in LIST.read_text().splitlines()
              if line.startswith("#") and not line.startswith("# rustc ")]
    lines = header + [f"# {compiler}"] + [
        f"{c}\t{len(got[(c, f)][0]) if got[(c, f)] else 0}\t"
        f"{got[(c, f)][1] if got[(c, f)] else 0}\t{f}" for (c, f) in rows]
    return "\n".join(lines) + "\n"


def listed():
    """The file's hot functions: (crate, function) -> (scopes, lines)."""
    rows = {}
    for line in LIST.read_text().splitlines():
        if not line.strip() or line.startswith("#"):
            continue
        crate, count, size, function = line.split("\t")
        rows[(crate, function)] = (int(count), int(size))
    return rows


def touch(root):
    """Every file `root` tracks made newer than anything built (as gate.py's
    `build` does). `target` is the gate's, which built the base last: cargo
    takes a path crate as built when its sources, at their paths in the
    package, are older than its build, wherever the build's sources lay. So
    the head's zgx-gpu was built on the base's zgx (#1937: a method the
    head added to zgx was missing from it, E0599)."""
    tracked = subprocess.run(["git", "ls-files", "-z"], cwd=root, capture_output=True,
                             check=True).stdout
    for path in filter(None, tracked.split(b"\0")):
        file = pathlib.Path(root) / path.decode()
        if file.is_file():
            os.utime(file)


def measure(root, target, wanted, crates=CRATES):
    """Each wanted function's inlined callees and lines, crate by crate."""
    touch(root)
    got = {}
    for crate in crates:
        if not any(c == crate for c, _ in wanted):
            continue
        found, lines = scopes(mir(root, crate, target))
        for (c, function) in wanted:
            if c == crate:
                got[(c, function)] = (found[function], lines[function]) if function in found else None
    return got


def at(root, ref, target, wanted, crates):
    """`wanted` as the tree at `ref` has them (a worktree of its own)."""
    tree = target / "inline-scopes-base"
    subprocess.run(["git", "worktree", "add", "-q", "--detach", str(tree), ref], cwd=root, check=True)
    try:
        return measure(tree, target, wanted, crates)
    finally:
        subprocess.run(["git", "worktree", "remove", "--force", str(tree)], cwd=root, check=True)


def verdict(count, size, scopes_now, lines_now):
    """Why a function recorded with `count` scopes and `size` lines fails
    with these now (empty: it passes)."""
    why = []
    if scopes_now < count - SLACK:
        why.append(f"inlines {scopes_now} scopes, {count} recorded")
    if lines_now > size * GROWTH and lines_now > size + GROWTH_LINES:
        why.append(f"grew to {lines_now} lines from {size}")
    return "; ".join(why)


def changed(before, after):
    """Callees inlined in one and not the other, by name and count."""
    gained = collections.Counter(after) - collections.Counter(before)
    lost = collections.Counter(before) - collections.Counter(after)
    shown = lambda c: ", ".join(f"{name} x{n}" if n > 1 else name for name, n in c.most_common(8))
    return shown(gained), shown(lost)


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("command", nargs="?", default="check", choices=["check", "record"])
    parser.add_argument("--root", type=pathlib.Path, default=HERE.parents[1], help="the tree to build")
    parser.add_argument("--target", type=pathlib.Path, help="its CARGO_TARGET_DIR (default: ROOT/target)")
    parser.add_argument("--list", type=pathlib.Path, help="the list to read and write (default: next to this)")
    parser.add_argument("--base", help="on a failure, the ref to name gained and lost callees against")
    parser.add_argument("--triple", help="the target to build for (default: the host's; CI's is x86_64 Linux)")
    args = parser.parse_args()
    global LIST, TRIPLE
    if args.list:
        LIST = args.list
    TRIPLE = args.triple
    target = args.target or args.root / "target"
    rows = listed()
    got = measure(args.root, target, rows)
    missing = [key for key, callees in got.items() if callees is None]
    compiler = rustc(args.root)
    if args.command == "record":
        LIST.write_text(table(rows, got, compiler))
        for key in missing:
            print(f"not found (renamed or removed?): {key[0]} {key[1]}")
        print(f"inline scopes: recorded {len(rows)} functions")
        return
    if recorded_by() != compiler:
        note = "::warning::" if os.environ.get("GITHUB_ACTIONS") else ""
        print(f"{note}inline scopes: counts recorded by {recorded_by()}, built by {compiler}: "
              f"not checked. Record them with it ({LIST.name}):")
        print(table(rows, got, compiler), end="")
        return
    failed = []
    for (crate, function), (count, size) in rows.items():
        now = got[(crate, function)]
        if now is None:
            failed.append((crate, function, "not found (renamed? record it again)"))
            continue
        callees, lines = now
        why = verdict(count, size, len(callees), lines)
        if why:
            failed.append((crate, function, why))
        elif (len(callees), lines) != (count, size):
            print(f"{crate} {function}: {count} -> {len(callees)} scopes, {size} -> {lines} lines (record it again)")
    if failed:
        print("inline scopes: a hot function inlines differently than it did:", file=sys.stderr)
        for crate, function, why in failed:
            print(f"  {crate} {function}: {why}", file=sys.stderr)
        if args.base:
            keys = [(c, f) for c, f, _ in failed]
            before = at(args.root, args.base, target, keys, sorted({c for c, _ in keys}))
            for key in keys:
                if before.get(key) and got.get(key):
                    gained, lost = changed(before[key][0], got[key][0])
                    print(f"  {key[1]}, against {args.base}: inlines now {gained or '-'}; no longer {lost or '-'}",
                          file=sys.stderr)
        sys.exit(1)
    print(f"inline scopes: {len(rows)} hot functions inline as recorded")


if __name__ == "__main__":
    main()
