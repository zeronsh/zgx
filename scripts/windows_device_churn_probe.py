#!/usr/bin/env python3
"""Renderers made, drawn with and dropped on many threads at once, on a
Direct3D 12 GPU: with a device each, NVIDIA's driver hung the process in
the kernel (docs/validation.md, "NVIDIA on Windows"); zgx shares one device
among a process's contexts there, and none may hang.

Runs, each in a Job Object of its own (closing it ends everything the run
started) and timed out at --timeout seconds, a hang counted:

- `stress`: zgx-gpu's `independent_device_lifecycle_stress` (opt-in, 24
  threads, each making, drawing with and dropping renderers);
- `render`: zgx-video's render tests, twenty at once beside their ffmpeg
  encoders (ffmpeg must be on PATH, or the encoding tests skip).

Refuses where the only adapters are software (WARP, the Basic Render
Driver), which never hung.

    cargo test -p zgx-gpu --test pixels --locked --no-run
    cargo test -p zgx-video --test render --locked --no-run
    python scripts/windows_device_churn_probe.py [--runs 20] [--only stress|render]
"""
import argparse
import ctypes
import ctypes.wintypes as W
import glob
import os
import pathlib
import shutil
import subprocess
import sys
import time

ROOT = pathlib.Path(__file__).resolve().parents[1]
KILL_ON_JOB_CLOSE = 0x2000
EXTENDED_LIMIT_INFORMATION = 9

CASES = {
    "stress": ("pixels", ["--ignored", "--exact", "independent_device_lifecycle_stress"]),
    "render": ("render", []),
}


def kernel32():
    k = ctypes.WinDLL("kernel32", use_last_error=True)
    k.CreateJobObjectW.restype = W.HANDLE
    k.CreateJobObjectW.argtypes = [ctypes.c_void_p, W.LPCWSTR]
    k.SetInformationJobObject.argtypes = [W.HANDLE, ctypes.c_int, ctypes.c_void_p, W.DWORD]
    k.AssignProcessToJobObject.argtypes = [W.HANDLE, W.HANDLE]
    k.CloseHandle.argtypes = [W.HANDLE]
    return k


def job(k):
    """A job that ends what it holds when closed."""
    handle = k.CreateJobObjectW(None, None)
    limits = ctypes.create_string_buffer(144)  # JOBOBJECT_EXTENDED_LIMIT_INFORMATION
    ctypes.c_uint32.from_buffer(limits, 16).value = KILL_ON_JOB_CLOSE
    if not k.SetInformationJobObject(handle, EXTENDED_LIMIT_INFORMATION, limits, 144):
        raise OSError(ctypes.get_last_error(), "SetInformationJobObject")
    return handle


def hardware_adapters():
    """The display adapters that aren't software ones."""
    out = subprocess.run(
        ["powershell", "-NoProfile", "-Command",
         "Get-CimInstance Win32_VideoController | ForEach-Object { $_.Name }"],
        capture_output=True, text=True, check=False).stdout
    names = [line.strip() for line in out.splitlines() if line.strip()]
    return [name for name in names if "Basic Render" not in name and "Basic Display" not in name]


def newest(target, name):
    found = glob.glob(str(target / "deps" / f"{name}-*.exe"))
    found += glob.glob(str(target / "build" / "*" / "*" / "out" / f"{name}-*.exe"))
    return max(found, key=os.path.getmtime) if found else None


def run_once(k, exe, args, timeout):
    """'ok', 'failed' or 'hung', and the run's output."""
    handle = job(k)
    env = dict(os.environ, ZGX_RENDERER="gpu")
    env.pop("WGPU_FORCE_FALLBACK_ADAPTER", None)
    proc = subprocess.Popen([exe, *args], env=env, cwd=str(ROOT),
                            stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    if not k.AssignProcessToJobObject(handle, int(proc._handle)):
        k.CloseHandle(handle)
        proc.kill()
        raise OSError(ctypes.get_last_error(), "AssignProcessToJobObject")
    try:
        out, _ = proc.communicate(timeout=timeout)
        outcome = "ok" if proc.returncode == 0 else "failed"
    except subprocess.TimeoutExpired:
        k.CloseHandle(handle)  # ends the run and everything it started
        handle = None
        out, _ = proc.communicate()
        outcome = "hung"
    if handle:
        k.CloseHandle(handle)
    text = out.decode(errors="replace")
    if outcome == "ok" and "test result: ok. 0 passed" in text:
        outcome = "failed"  # nothing ran: a filter matched no test
    return outcome, text


def main():
    if sys.platform != "win32":
        sys.exit("windows_device_churn_probe runs on Windows")
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--runs", type=int, default=20)
    parser.add_argument("--timeout", type=int, default=180)
    parser.add_argument("--target", default=str(ROOT / "target" / "debug"))
    parser.add_argument("--only", choices=sorted(CASES))
    args = parser.parse_args()
    adapters = hardware_adapters()
    if not adapters:
        sys.exit("no Direct3D 12 GPU here (software adapters only): nothing to probe")
    print("adapters:", ", ".join(adapters), flush=True)
    if not shutil.which("ffmpeg") and args.only != "stress":
        print("warning: no ffmpeg on PATH: the render tests' encoding skips", flush=True)
    k = kernel32()
    target = pathlib.Path(args.target)
    cases = [args.only] if args.only else list(CASES)
    hung = 0
    for case in cases:
        name, case_args = CASES[case]
        exe = newest(target, name)
        if not exe:
            sys.exit(f"no {name} test binary under {target}: build it first (see --help)")
        counts = {"ok": 0, "failed": 0, "hung": 0}
        for run in range(1, args.runs + 1):
            started = time.monotonic()
            outcome, text = run_once(k, exe, case_args, args.timeout)
            counts[outcome] += 1
            if outcome != "ok":
                print(f"{case} run {run}: {outcome} after {time.monotonic() - started:.0f} s", flush=True)
                for line in text.splitlines():
                    if "running for over" in line or "panicked" in line or line.startswith("test result"):
                        print("  " + line, flush=True)
        hung += counts["hung"]
        print(f"RESULT {case}: {counts['hung']} of {args.runs} hung, {counts['failed']} failed ({exe})", flush=True)
    sys.exit(1 if hung else 0)


if __name__ == "__main__":
    main()
