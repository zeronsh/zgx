#!/usr/bin/env python3
"""Which Actions cache entries to delete (.github/workflows/ci-caches.yml).

    gh cache list --limit 1000 --json id,key,ref,createdAt | prune_caches.py

Prints `<id> <why>` for each entry to delete:

  - entries saved from any ref but main (pull requests, tags): no other ref
    reads them, and they push main's out of the repository's 10 GB;
  - Swatinem/rust-cache entries (`v0-rust-<job>-<os>-<arch>-<env>-<lock>`)
    that a newer entry of the same job, OS and architecture on main
    supersedes. A run restores the newest of them anyway (its own exact key
    aside), and each is 0.8 to 1.7 GB: two of each filled the 10 GB, and
    GitHub then evicted whatever was used least, perf's kept binaries first;
  - perf's kept binaries (`gate-bin-<sha>`, about 200 MB each, one for most
    pushes to main) but the newest KEEP_GATE_BIN: pull requests measure
    against recent main commits (gate_base.py).
"""

from __future__ import annotations

import json
import re
import sys

MAIN = "refs/heads/main"
RUST = re.compile(r"(v0-rust-.+)-[0-9a-f]{8}-[0-9a-f]{8}")
GATE_BIN = "gate-bin-"
# A few hours of main's pushes on a busy day: about 1.6 GB.
KEEP_GATE_BIN = 8


def family(key: str) -> str | None:
    """A rust-cache entry's job, OS and architecture: its key without the
    environment (compiler) and lockfile hashes. None for other caches."""
    match = RUST.fullmatch(key)
    return match.group(1) if match else None


def to_delete(entries: list[dict]) -> list[tuple[int, str]]:
    out: list[tuple[int, str]] = []
    newest: dict[str, dict] = {}
    gate_bins = 0
    for entry in sorted(entries, key=lambda e: e["createdAt"], reverse=True):
        if entry["ref"] != MAIN:
            out.append((entry["id"], f"saved from {entry['ref']}"))
            continue
        if entry["key"].startswith(GATE_BIN):
            gate_bins += 1
            if gate_bins > KEEP_GATE_BIN:
                out.append((entry["id"], f"older than the newest {KEEP_GATE_BIN} perf binaries"))
            continue
        fam = family(entry["key"])
        if fam is None:
            continue
        if fam in newest:
            out.append((entry["id"], f"superseded by {newest[fam]['key']}"))
        else:
            newest[fam] = entry
    return out


def main() -> int:
    for id_, why in to_delete(json.load(sys.stdin)):
        print(id_, why)
    return 0


if __name__ == "__main__":
    sys.exit(main())
