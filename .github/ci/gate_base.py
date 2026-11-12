#!/usr/bin/env python3
"""Which of main's kept perf binaries the perf job can measure against.

    gate_base.py pick BASE            key=... and from=... lines for $GITHUB_OUTPUT
    gate_base.py adopt DIR FROM BASE  restored binaries of FROM, stamped as BASE's
    gate_base.py needed BEFORE AFTER  build=true|false: whether a push to main
                                      needs binaries of its own (perf-base.yml)

The perf job (ci.yml) counts a pull request's binaries against its base's
(scripts/perf/gate.py). main keeps commits' binaries in the Actions cache
(`gate-bin-<sha>`: perf-base.yml for each push that changes what perf builds,
CI's main runs for theirs), so a pull request builds its own side alone. A
pull request based on a commit with none built its base too: about 24 of the
job's 70 minutes.

Binaries built from two commits that differ in nothing perf builds or runs
(plan.perf_builds) are the same binaries. So when BASE has none kept, `pick`
takes the newest kept commit no such change separates from BASE, and `adopt`
stamps them as BASE's (gate.py's `stamp`: the commit, a newline, then
`rustc -vV`; another compiler still fails gate.py's check, and it builds).
Anything unexpected picks nothing: gate.py builds the base, as before.

A pull request's run restores from its own cache scope first, and its earlier
commits can write there (an edited ci.yml saves anything): a `gate-bin-<sha>`
of its own making would be measured as main's. So outside main, `pick` skips a
commit whose key has an entry in the run's own scope (`shadowed`), as if main
kept none for it; if that can't be checked, it picks nothing.
"""

from __future__ import annotations

import os
import pathlib
import subprocess
import sys
from urllib.parse import quote

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import plan  # noqa: E402

PREFIX = "gate-bin-"
CANDIDATES = 20  # newest kept commits to try


def kept(repo: str) -> list[str]:
    """Commits with binaries kept on main, newest first."""
    data = plan.api(f"/repos/{repo}/actions/caches?key={PREFIX}&ref=refs/heads/main"
                    f"&sort=created_at&direction=desc&per_page=100")
    return [c["key"][len(PREFIX):] for c in data.get("actions_caches", []) if c["key"].startswith(PREFIX)]


def shadowed(repo: str, ref: str, sha: str) -> bool:
    """Whether `ref`'s own cache scope has an entry under `sha`'s key, which a
    restore there would take instead of main's."""
    data = plan.api(f"/repos/{repo}/actions/caches?key={PREFIX}{sha}&ref={quote(ref, safe='')}&per_page=100")
    return any(c.get("key") == f"{PREFIX}{sha}" for c in data.get("actions_caches", []))


def own_scope() -> str | None:
    """The run's own cache scope when it isn't main's: a pull request's merge ref."""
    ref = os.environ.get("GITHUB_REF", "")
    return ref if ref and ref != "refs/heads/main" else None


def changed(a: str, b: str) -> list[str] | None:
    result = subprocess.run(["git", "diff", "--name-only", "--no-renames", a, b],
                            capture_output=True, text=True)
    return result.stdout.split() if result.returncode == 0 else None


def choose(base: str, candidates: list[str], diff=None, shadowed=None) -> str | None:
    """The base's own kept binaries, or the newest kept commit's that nothing perf
    builds separates from it; never one `shadowed` (the run's own scope has its key)."""
    diff = diff or changed
    shadowed = shadowed or (lambda sha: False)
    if base in candidates and not shadowed(base):
        return base
    for sha in candidates[:CANDIDATES]:
        if sha == base:
            continue
        paths = diff(sha, base)
        if paths is not None and not any(plan.perf_builds(p) for p in paths) and not shadowed(sha):
            return sha
    return None


def adopt(directory: pathlib.Path, source: str, base: str) -> bool:
    stamp = directory / "built-from"
    if not stamp.is_file():
        return False
    commit, newline, rest = stamp.read_text().partition("\n")
    if commit != source or not newline:
        return False
    stamp.write_text(base + "\n" + rest)
    return True


def needed(before: str, after: str, compare=None) -> bool:
    """Whether a push to main changes what perf builds (perf-base.yml): when it
    doesn't, the commit before's binaries stand for it. Unsure: yes."""
    compare = compare or (lambda a, b: plan.compare_files(os.environ["GITHUB_REPOSITORY"], a, b))
    try:
        paths = compare(before, after)
    except (OSError, KeyError, ValueError) as error:
        print(f"::warning::could not compare {before[:10]}...{after[:10]} ({error})", file=sys.stderr)
        return True
    return paths is None or any(plan.perf_builds(p) for p in paths)


def main() -> int:
    if len(sys.argv) == 4 and sys.argv[1] == "needed":
        build = needed(sys.argv[2], sys.argv[3])
        if not build:
            print("Nothing perf builds changed: the commit before's binaries stand for this one", file=sys.stderr)
        print(f"build={'true' if build else 'false'}")
        return 0
    if len(sys.argv) == 3 and sys.argv[1] == "pick":
        base = sys.argv[2]
        repo, scope = os.environ["GITHUB_REPOSITORY"], own_scope()
        try:
            sha = choose(base, kept(repo), shadowed=(lambda s: shadowed(repo, scope, s)) if scope else None)
        except (OSError, KeyError, ValueError) as error:
            print(f"::warning::could not list main's kept perf binaries, or this run's own ({error}); "
                  "building the base",
                  file=sys.stderr)
            return 0
        if sha is None:
            print(f"No kept perf binaries match {base[:10]}: building the base", file=sys.stderr)
            return 0
        if sha != base:
            print(f"Base {base[:10]}: measured with {sha[:10]}'s binaries (nothing perf builds differs)",
                  file=sys.stderr)
        print(f"key={PREFIX}{sha}")
        print(f"from={sha}")
        return 0
    if len(sys.argv) == 5 and sys.argv[1] == "adopt":
        directory, source, base = pathlib.Path(sys.argv[2]), sys.argv[3], sys.argv[4]
        if adopt(directory, source, base):
            print(f"{source[:10]}'s binaries stand for {base[:10]}'s")
        else:
            print(f"::warning::the restored binaries are not {source[:10]}'s; building the base")
        return 0
    print(__doc__, file=sys.stderr)
    return 2


if __name__ == "__main__":
    sys.exit(main())
