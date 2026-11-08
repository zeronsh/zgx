#!/usr/bin/env python3
"""A long session of a zgx app on a simulator: memory and CPU each minute,
and objects that pile up. The app drives itself (`ZGX_APP_ARGS=soak`: the
showcase's and the mail example's `soak` modes), so nothing is posted to it.

    python3 scripts/ios/sim.py run -p zgx-run --example ios_app --release \\
        --name soak_showcase --resource crates/zgx-script/examples/ios_showcase --install-only
    python3 scripts/ios/soak.py dev.zgx.ios.soak-showcase --minutes 60
    python3 scripts/ios/soak.py dev.zgx.ios.soak-showcase --minutes 5 --only push

It samples `footprint` (phys_footprint, exact bytes) and CPU every minute,
counts objects by class with `heap` after the first minute and at the end,
and fails when one of `--watch` grew by more than `--slack` (each class is a
UIKit or Metal object a cycle must give back: `_UIUpdateLinkViewInteraction`
piled up by one per un-hidden native view until `native_host/ios.rs` hid
them by their layers), or when the app died. Every simulator call has a time
limit: one that hangs stops the run, and nothing answers a dialog.
"""

import argparse
import re
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
OUT = ROOT / "target" / "ios-soak"
WATCH = ["_UIUpdateLinkViewInteraction", "UIVisualEffectView", "UISwitch", "__NSMallocBlock__"]


def call(args, timeout, **kwargs):
    try:
        return subprocess.run(args, capture_output=True, text=True, timeout=timeout, **kwargs)
    except subprocess.TimeoutExpired:
        sys.exit(f"{' '.join(args[:3])} took over {timeout} s: stopped (a hung simulator or a dialog).")


def device():
    listing = call(["xcrun", "simctl", "list", "devices", "available"], 30).stdout
    for line in listing.splitlines():
        if "zgx-ios-lab (" in line:
            return line.split("(")[1].split(")")[0]
    sys.exit("No zgx-ios-lab simulator.")


def counts(pid, classes):
    out = call(["heap", str(pid)], 180).stdout
    found = dict.fromkeys(classes, 0)
    for line in out.splitlines():
        m = re.match(r"\s*(\d+)\s+\d+\s+[\d.]+\s+(\S+)", line)
        if m and m.group(2) in found:
            found[m.group(2)] += int(m.group(1))
    return found


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("bundle")
    parser.add_argument("--minutes", type=int, default=60)
    parser.add_argument("--only", help="the soak's one group of steps (the showcase's push, sheet, tabs…)")
    parser.add_argument("--watch", action="append", help="a class that must not pile up (default: %s)" % ", ".join(WATCH))
    parser.add_argument("--slack", type=int, default=8, help="growth a watched class may show")
    args = parser.parse_args()
    watch = args.watch or WATCH
    udid = device()
    OUT.mkdir(parents=True, exist_ok=True)
    log = OUT / f"{args.bundle}.log"
    call(["xcrun", "simctl", "terminate", udid, args.bundle], 30)
    env = dict(__import__("os").environ, SIMCTL_CHILD_ZGX_APP_ARGS="soak" + (f" {args.only}" if args.only else ""))
    launched = call(["xcrun", "simctl", "launch", f"--stdout={log}", f"--stderr={log}", udid, args.bundle], 30, env=env)
    if launched.returncode != 0:
        sys.exit(f"launch failed: {launched.stderr.strip()}")
    pid = int(launched.stdout.split()[-1])
    print("minute\tcycles\tphys_footprint\tcpu")
    first = None
    for minute in range(args.minutes + 1):
        time.sleep(60)
        if call(["kill", "-0", str(pid)], 10).returncode != 0:
            sys.exit(f"the app died at minute {minute}")
        footprint = call(["footprint", "-f", "bytes", str(pid)], 60).stdout
        found = re.search(r"phys_footprint:\s+(\d+)", footprint)
        cpu = call(["ps", "-o", "%cpu=", "-p", str(pid)], 10).stdout.strip()
        cycles = log.read_text(errors="replace").count("SOAK cycle") if log.exists() else 0
        print(f"{minute}\t{cycles}\t{found.group(1) if found else '?'}\t{cpu}", flush=True)
        if minute == 1:
            first = counts(pid, watch)
    last = counts(pid, watch)
    call(["xcrun", "simctl", "terminate", udid, args.bundle], 30)
    grew = {name: last[name] - first[name] for name in watch if last[name] - first[name] > args.slack}
    for name in watch:
        print(f"{name}: {first[name]} -> {last[name]}")
    if grew:
        sys.exit(f"piled up: {grew}")
    print("ok: nothing watched piled up, and the app lived")


if __name__ == "__main__":
    main()
