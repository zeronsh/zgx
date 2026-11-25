#!/usr/bin/env python3
"""What scripts/release_check.sh lists by hand is what the tree holds.

    python3 scripts/check_release_lists.py

- Its `forks` are the vendor/ crates named zgx-*, each after the forks it
  depends on (zgx-swash was left out, which zgx-cosmic-text needs).
- Its fresh app's crates are zgx-script's published dependencies and theirs,
  optional ones too: Cargo resolves those whatever is on.
- Every published workspace crate is under a path it looks for crates in
  (crates/zgx-module/macros is one deeper than the rest).
- A fork's committed lockfile has the fork's version: packaging writes it
  otherwise, and the check stops at a dirty tree (zgx-harfrust's, 0.5.3 at
  0.5.4).
"""
import json
import pathlib
import re
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent


def field(text, name):
    return re.search(rf'(?m)^{name} = "([^"]+)"', text).group(1)


def problems():
    found = []
    script = (ROOT / "scripts/release_check.sh").read_text(encoding="utf-8")
    forks = re.search(r'(?m)^forks="([^"]+)"', script).group(1).split()
    vendored = {}
    for manifest in sorted(ROOT.glob("vendor/*/Cargo.toml")):
        text = manifest.read_text(encoding="utf-8")
        if not field(text, "name").startswith("zgx-"):
            continue
        fork = manifest.parent.name
        vendored[fork] = text
        tracked = subprocess.run(["git", "ls-files", "--error-unmatch", f"vendor/{fork}/Cargo.lock"], cwd=ROOT,
                                 stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL).returncode == 0
        if tracked:
            lock = (manifest.parent / "Cargo.lock").read_text(encoding="utf-8")
            locked = re.search(rf'name = "{field(text, "name")}"\nversion = "([^"]+)"', lock)
            if not locked or locked.group(1) != field(text, "version"):
                found.append(f"vendor/{fork}/Cargo.lock has {locked and locked.group(1)}, "
                             f"its manifest {field(text, 'version')}")
    for fork in sorted(set(vendored) ^ set(forks)):
        found.append(f"fork {fork}: in {'vendor/' if fork in vendored else 'release_check.sh'} alone")
    for i, fork in enumerate(forks):
        for needed in re.findall(r'package = "zgx-([a-z-]+)"', vendored.get(fork, "")):
            if needed in vendored and needed not in forks[:i]:
                found.append(f"fork {fork} comes before {needed}, which it depends on")
            if needed in vendored and not re.search(rf'"\$fork" = {fork} \]; then deps="[^"]*\b{needed}\b', script):
                found.append(f"fork {fork} isn't given {needed} as it is packaged")
    packages = {p["name"]: p for p in json.loads(subprocess.check_output(
        ["cargo", "metadata", "--format-version", "1", "--no-deps", "--locked"], cwd=ROOT))["packages"]}
    published = {name for name, p in packages.items() if p["publish"] is None}
    if script.count('"/crates/" in ') + script.count('"/crates/" not in ') != 2:
        found.append("release_check.sh doesn't find crates by /crates/ in both its places")
    for name in sorted(published):
        if "/crates/" not in packages[name]["manifest_path"].replace("\\", "/"):
            found.append(f"{name} is published from outside crates/")
    fresh = set(re.search(r"for crate in (zgx [^;]+); do", script).group(1).split())
    needs, left = set(), ["zgx-script", "zgx-desktop", "zgx-run", "zgx-components"]
    while left:
        name = left.pop()
        if name not in needs:
            needs.add(name)
            left += [d["name"] for d in packages[name]["dependencies"]
                     if d["name"] in published and d["kind"] != "dev"]
    for name in sorted(needs - fresh):
        found.append(f"the fresh app isn't given {name}")
    return found


if __name__ == "__main__":
    found = problems()
    for problem in found:
        print(problem, file=sys.stderr)
    if found:
        sys.exit(1)
    print("release_check.sh's lists are the tree's")
