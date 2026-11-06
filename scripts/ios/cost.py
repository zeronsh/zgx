#!/usr/bin/env python3
"""Run the ios_cost example in each mode on a simulator and report what its
process costs: CPU over a window, memory, and time to its first frame. The
GPU and the compositor are the Mac's there, so this says nothing about them,
and CPU only roughly what a phone's would be: compare modes with each other."""

import argparse
import re
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts" / "ios"))
import sim  # noqa: E402


def cpu_seconds(pid: int) -> float:
    out = subprocess.run(["ps", "-o", "cputime=", "-p", str(pid)], capture_output=True, text=True).stdout.strip()
    parts = out.replace("-", ":").split(":")
    seconds = 0.0
    for part in parts:
        seconds = seconds * 60 + float(part)
    return seconds


def footprint_mb(pid: int) -> float:
    out = subprocess.run(["footprint", str(pid)], capture_output=True, text=True).stdout
    match = re.search(r"phys_footprint:\s*([\d.]+)\s*(KB|MB|GB)", out) or re.search(
        r"Footprint:\s*([\d.]+)\s*(KB|MB|GB)", out
    )
    if not match:
        return -1.0
    return float(match.group(1)) * {"KB": 1 / 1024, "MB": 1, "GB": 1024}[match.group(2)]


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--seconds", type=float, default=15.0)
    parser.add_argument("--repeat", type=int, default=3)
    parser.add_argument("--rows", type=int, default=200)
    parser.add_argument("--release", action="store_true")
    parser.add_argument("cases", nargs="*", default=["plain", "drawn", "hosted", "plain:1", "plain:4"])
    args = parser.parse_args()
    environment = {k: v for k, v in sim.os.environ.items() if k != "SDKROOT"}
    build = ["cargo", "build", "-p", "zgx-desktop", "--example", "ios_cost", "--target", sim.TARGET, "--locked"]
    if args.release:
        build.append("--release")
    subprocess.run(build, check=True, cwd=ROOT, env=dict(environment, CARGO_PROFILE_DEV_DEBUG="0"))
    profile = "release" if args.release else "debug"
    binary = ROOT / "target" / sim.TARGET / profile / "examples" / "ios_cost"
    app = sim.bundle(binary, "ios_cost", [])
    udid = sim.device()
    identifier = "sh.zeron.zgx.ios.ios-cost"
    sim.simctl("install", udid, str(app))
    print(f"{'case':34}{'CPU % core':>12}{'memory MB':>12}{'first frame ms':>16}")
    for case in args.cases:
        mode, _, rest = case.partition(":")
        panes, _, extra = rest.partition(":")
        rows = []
        for _ in range(args.repeat):
            sim.simctl("terminate", udid, identifier, check=False)
            log = ROOT / "target" / "ios" / "ios_cost.log"
            log.unlink(missing_ok=True)
            launch = dict(
                environment,
                SIMCTL_CHILD_COST_MODE=mode,
                SIMCTL_CHILD_COST_GLASS=panes or "0",
                SIMCTL_CHILD_COST_ROWS=str(args.rows),
                SIMCTL_CHILD_ZGX_PROFILE="1",
            )
            if extra:
                key, _, value = extra.partition("=")
                launch["SIMCTL_CHILD_" + key] = value
            out = subprocess.run(
                ["xcrun", "simctl", "launch", f"--stdout={log}", f"--stderr={log}", udid, identifier],
                env=launch,
                capture_output=True,
                text=True,
            ).stdout
            pid = int(out.strip().rsplit(" ", 1)[1])
            time.sleep(5)
            before = cpu_seconds(pid)
            began = time.time()
            time.sleep(args.seconds)
            spent = cpu_seconds(pid) - before
            elapsed = time.time() - began
            memory = footprint_mb(pid)
            first = re.search(r"([\d.]+) ms: first frame presented", log.read_text()) if log.exists() else None
            rows.append((100 * spent / elapsed, memory, float(first.group(1)) if first else -1))
            sim.simctl("terminate", udid, identifier, check=False)
        rows.sort()
        cpu, memory, first = rows[len(rows) // 2]
        print(f"{case:34}{cpu:12.1f}{memory:12.1f}{first:16.0f}", flush=True)
    return 0


if __name__ == "__main__":
    sys.exit(main())
