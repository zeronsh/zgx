#!/usr/bin/env python3
"""The previous release's Luau apps on this one (the release check's step 4
at a breaking version, where the previous crates can't be built against this
release's: they ask for versions Cargo won't give them).

    python3 scripts/previous_scripts.py --previous 0.1.10 --target target

The previous release's own `zgx` (from crates.io) writes each of its
templates, and its package holds its examples. Each is run by both `zgx
test`s, with a test of its own mounting it added to the tests it came with:
one that passes on the previous release and fails on this one fails here.
One that fails on both (an example needing its Rust host) says so and
counts for nothing: at least one must pass on both.
"""
import argparse
import pathlib
import shutil
import subprocess
import sys
import tarfile
import tempfile
import urllib.request

ROOT = pathlib.Path(__file__).resolve().parent.parent
TEMPLATES = ("basic", "router", "list", "form", "plugins")


def verdict(passed):
    """passed: {name: (on the previous release, on this one)} -> (broke, untested)."""
    broke = sorted(n for n, (old, new) in passed.items() if old and not new)
    untested = sorted(n for n, (old, _) in passed.items() if not old)
    return broke, untested


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--previous", required=True)
    parser.add_argument("--target", required=True)
    args = parser.parse_args()
    target = pathlib.Path(args.target).resolve()
    exe = ".exe" if sys.platform == "win32" else ""
    root = target / "previous-zgx"
    subprocess.run(
        ["cargo", "install", "zgx-script", "--version", args.previous, "--bin", "zgx", "--locked",
         "--debug", "--root", str(root), "--target-dir", str(target / "previous-build")],
        check=True, cwd=ROOT.parent)
    subprocess.run(["cargo", "build", "-p", "zgx-script", "--bin", "zgx", "--locked",
                    "--target-dir", str(target)], check=True, cwd=ROOT)
    old, new = root / "bin" / f"zgx{exe}", target / "debug" / f"zgx{exe}"
    url = f"https://static.crates.io/crates/zgx-script/zgx-script-{args.previous}.crate"
    request = urllib.request.Request(url, headers={"User-Agent": "zgx-release-check"})
    passed = {}
    with tempfile.TemporaryDirectory() as scratch:
        scratch = pathlib.Path(scratch)
        crate = scratch / "crate.tar.gz"
        with urllib.request.urlopen(request, timeout=60) as response:
            crate.write_bytes(response.read())
        with tarfile.open(crate) as tar:
            # Plain files under its own directory only.
            members = [m for m in tar.getmembers() if m.isfile() and ".." not in pathlib.PurePosixPath(m.name).parts
                       and not m.name.startswith("/")]
            tar.extractall(scratch, members=members)
        apps = []
        for template in TEMPLATES:
            for i18n in ((), ("--i18n",)):
                name = template + "".join(i18n)
                subprocess.run([str(old), "new", str(scratch / name), "--template", template, *i18n],
                               check=True, stdout=subprocess.DEVNULL)
                apps.append((name, scratch / name))
        examples = scratch / f"zgx-script-{args.previous}" / "examples"
        for app in sorted(examples.glob("*/app.luau")):
            apps.append((f"examples/{app.parent.name}", app.parent))
        for name, directory in apps:
            (directory / "zz_mounts.test.luau").write_text('test("mounts", function() mount() end)\n')
            ran = []
            for zgx in (old, new):
                try:
                    done = subprocess.run([str(zgx), "test", str(directory)], cwd=directory, timeout=300,
                                          stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
                except subprocess.TimeoutExpired:
                    ran.append(False)
                    continue
                ran.append(done.returncode == 0)
                if zgx == new and ran[0] and not ran[1]:
                    print(done.stdout.decode(errors="replace"))
            passed[name] = tuple(ran)
    shutil.rmtree(target / "previous-build", ignore_errors=True)
    broke, untested = verdict(passed)
    both = sum(1 for pair in passed.values() if all(pair))
    print(f"{both} of {args.previous}'s apps pass on both; not tested (they fail on {args.previous} itself): "
          f"{', '.join(untested) or 'none'}")
    if broke:
        print(f"pass on {args.previous} and fail on this release: {', '.join(broke)}")
    if broke or not both:
        sys.exit(1)


if __name__ == "__main__":
    main()
