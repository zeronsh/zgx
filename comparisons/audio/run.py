#!/usr/bin/env python3
"""Runs zgx-audio's mixer against kira and miniaudio, as README.md says.

macOS only (instructions are read with thread_selfcounts). Fetches
miniaudio.h at its pinned tag and checks its hash, builds both harnesses,
writes every engine's 10 s of output to out/ and compares them, then runs
each cost arm and prints instructions a voice a frame.

    python3 comparisons/audio/run.py            # everything (about 4 minutes)
    python3 comparisons/audio/run.py --sound    # only the same-work check
"""
import hashlib
import os
import subprocess
import sys
import urllib.request
from pathlib import Path

HERE = Path(__file__).resolve().parent
MINIAUDIO_TAG = "0.11.25"
MINIAUDIO_SHA256 = "ac7af4de748b7e26b777f37e01cee313a308a7296a3eb080e2906b320cc55c89"
HEADER = HERE / "miniaudio" / "miniaudio.h"
BENCH = HERE / "miniaudio" / "bench"
OUT = HERE / "out"

# (voices, rate, block, pan, ramps, limiter, interpolation, layout)
ARMS = [
    "64 48000 512 spread none on cubic buses",
    "64 48000 512 spread none off cubic buses",
    "64 48000 128 spread none off cubic buses",
    "64 48000 1024 spread none off cubic buses",
    "64 48000 512 center none off cubic buses",
    "64 44100 512 spread none off cubic buses",
    "64 44100 512 spread none off linear buses",
    "64 48000 512 spread gain off cubic buses",
    "64 48000 512 spread gainpan off cubic buses",
    "64 48000 512 spread all off cubic buses",
    "64 44100 512 spread all off cubic buses",
    "1 48000 512 spread none off cubic buses",
    "1 48000 512 spread none on cubic buses",
    # Where zgx-audio's cost goes: read but not mixed, voices on master.
    "64 48000 512 spread none off cubic silent",
    "64 48000 512 spread none off cubic master",
]
MINIAUDIO_ARMS = [
    "64 48000 512 spread none",
    "64 48000 512 center none",
    "64 48000 128 spread none",
    "64 48000 1024 spread none",
    "64 44100 512 spread none",
    "64 48000 512 spread gain",
    "1 48000 512 spread none",
]


def fetch_header():
    if HEADER.exists() and hashlib.sha256(HEADER.read_bytes()).hexdigest() == MINIAUDIO_SHA256:
        return
    url = f"https://raw.githubusercontent.com/mackron/miniaudio/{MINIAUDIO_TAG}/miniaudio.h"
    body = urllib.request.urlopen(url, timeout=60).read()
    digest = hashlib.sha256(body).hexdigest()
    if digest != MINIAUDIO_SHA256:
        sys.exit(f"miniaudio.h {MINIAUDIO_TAG} has hash {digest}, not {MINIAUDIO_SHA256}")
    HEADER.write_bytes(body)


def build():
    subprocess.run(["cargo", "build", "--release", "--locked"], cwd=HERE, check=True)
    subprocess.run(["clang", "-O3", "-std=c11", "-o", str(BENCH), str(HERE / "miniaudio" / "bench.c")], check=True)
    target = Path(os.environ.get("CARGO_TARGET_DIR", HERE / "target"))
    return target / "release" / "audio-compare"


def main():
    if sys.platform != "darwin":
        sys.exit("macOS only: instructions are read with thread_selfcounts")
    fetch_header()
    compare = build()
    OUT.mkdir(exist_ok=True)
    for rate in ("48000", "44100"):
        for pan in ("center", "spread"):
            subprocess.run([str(BENCH), "64", rate, "512", pan, "none", str(OUT / f"out-{rate}-{pan}.f32")], check=True)
    subprocess.run([str(compare), "sound", str(OUT)], check=True)
    if "--sound" in sys.argv:
        return
    for arm in ARMS:
        subprocess.run([str(compare), *arm.split()], check=True)
    for arm in MINIAUDIO_ARMS:
        subprocess.run([str(BENCH), *arm.split()], check=True)


if __name__ == "__main__":
    main()
