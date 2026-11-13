#!/usr/bin/env python3
"""Frames a window hands over while it's typed into and clicked fast, and
while its own timer streams frames (macOS): runs the `input_cadence`
example with `ZGX_PRESENT_LOG=1` and reads its `handed over` lines.

    cargo build --release -p zgx-desktop --example input_cadence
    python3 scripts/input_cadence_probe.py [BINARY] [--refresh-hz=120]

Three runs: input at 240 Hz (frames a second, which should be the display's
refresh rate, gaps of one refresh), a 60 Hz timer stream with no input
(gaps of the stream's period, never alternating short and long), and a
window at rest (no frames)."""
import os
import pathlib
import statistics
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parents[1]
BINARY = pathlib.Path(next((a for a in sys.argv[1:] if not a.startswith("--")), "")) if any(not a.startswith("--") for a in sys.argv[1:]) else ROOT / "target/release/examples/input_cadence"
SECONDS = 4.0
# The display's refresh rate (`--refresh-hz`), to tell one refresh's
# hand-overs from the next's.
REFRESH_HZ = float(next((a.split("=")[1] for a in sys.argv if a.startswith("--refresh-hz=")), 120))


def run(env):
    """The hand-over times (seconds) and whether each frame was live."""
    out = subprocess.run([str(BINARY)], env=dict(os.environ, ZGX_PRESENT_LOG="1", ZGX_SECONDS=str(SECONDS), **env),
                         capture_output=True, text=True, timeout=SECONDS + 30)
    frames = []
    for line in out.stderr.splitlines():
        parts = line.split()
        if len(parts) >= 4 and parts[0] == "PRESENT" and parts[2:4] == ["handed", "over"]:
            frames.append((float(parts[1]), "live=true" in parts))
    if "input_cadence ticks=" not in out.stderr:
        raise SystemExit(f"the example didn't finish:\n{out.stderr[-2000:]}")
    return frames


def refreshes(frames, period=1 / REFRESH_HZ):
    """The refreshes that showed a new frame: hand-overs less than half a
    refresh apart are one refresh's (input drawing at once can hand over two
    in one refresh, the later shown), each kept at its last."""
    shown = []
    for at, live in frames:
        if shown and at - shown[-1][0] < period / 2:
            shown[-1] = (at, live or shown[-1][1])
        else:
            shown.append((at, live))
    return shown


def report(name, frames, skip=0.7):
    """Refreshes a second that showed a new frame, over the run (its first
    `skip` seconds left out: the window mounting and showing), and the gaps
    between them."""
    if not frames:
        print(f"{name}: no frames handed over")
        return
    handed = len(frames)
    frames = refreshes(frames)
    start = frames[0][0] + skip
    kept = [f for f in frames if f[0] >= start]
    if len(kept) < 2:
        print(f"{name}: {len(frames)} frames, none after the first {skip} s")
        return
    span = kept[-1][0] - kept[0][0]
    gaps = [(b[0] - a[0]) * 1e3 for a, b in zip(kept, kept[1:])]
    live = sum(1 for _, on in kept if on)
    buckets = {"<6": 0, "6-11": 0, "11-14": 0, "14-20": 0, ">=20": 0}
    for g in gaps:
        key = "<6" if g < 6 else "6-11" if g < 11 else "11-14" if g < 14 else "14-20" if g < 20 else ">=20"
        buckets[key] += 1
    share = "  ".join(f"{k} ms {v / len(gaps) * 100:.0f}%" for k, v in buckets.items())
    print(f"{name}: {(len(kept) - 1) / span:.1f} refreshes/s showed a new frame over {span:.2f} s "
          f"({handed} hand-overs), {live} of {len(kept)} live; "
          f"gaps p50 {statistics.median(gaps):.2f} ms, p95 {sorted(gaps)[int(len(gaps) * .95)]:.2f} ms")
    print(f"    {share}")


def main():
    if not BINARY.exists():
        raise SystemExit(f"build it first: {BINARY}")
    report("input 240 Hz", run({"ZGX_INPUT_HZ": "240", "ZGX_STREAM_HZ": "0"}))
    report("stream 60 Hz", run({"ZGX_INPUT_HZ": "0", "ZGX_STREAM_HZ": "60"}))
    rest = run({"ZGX_INPUT_HZ": "0", "ZGX_STREAM_HZ": "0"})
    later = [f for f in rest if f[0] >= (rest[0][0] + 1.0 if rest else 0)]
    print(f"rest: {len(rest)} frames in all, {len(later)} after the first second")


if __name__ == "__main__":
    main()
