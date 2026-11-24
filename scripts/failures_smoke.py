#!/usr/bin/env python3
"""The failures example fails twice while it animates: a task panics (the
host contains it and says so), then its GPU device is lost (it makes a new
renderer). It must go on drawing and exit cleanly."""
import re
import subprocess
import sys

binary = sys.argv[1]
try:
    run = subprocess.run([binary, "--smoke-test"], capture_output=True, text=True, timeout=60)
except subprocess.TimeoutExpired:
    sys.exit("failures did not finish")
log = run.stdout + run.stderr
print(log)
if run.returncode != 0:
    sys.exit(f"failures exited {run.returncode}")
for expected in (
    "a task panics",
    "losing the GPU device",
    "drawing again on a new GPU renderer",
    "still running at frame",
):
    if expected not in log:
        sys.exit(f"missing: {expected!r}")
# Contained, and said where: the panic's file and line lead its message.
if not re.search(r"contained a panic \(\S*failures\.rs:\d+: a task failed on purpose\)", log):
    sys.exit("missing: the contained panic, with where it was (failures.rs:<line>: ...)")
if "panicking repeatedly" in log:
    sys.exit("it gave up")
print("PASS: the app contained a panic and recovered from a lost GPU device")
