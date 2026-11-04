#!/usr/bin/env python3
"""A popup's glass on Windows blurs what its window drew under it
(`GpuRenderer::set_backdrop`), in a real popup window of zgx-run.

    python scripts/windows_popup_glass_probe.py target/release/zgx-run.exe [--seconds 10]

A Luau app shows hard stripes with a popover over them, open from the start,
its fill translucent. Read from the popup window's own capture, which holds
that window's pixels alone: with `backdrop_blur` its middle row is even and
brighter than the fill (the stripes, blurred); without, it is only the fill.
Past its rounded corners the popup keeps nothing of the window: they stay
empty, where a copy of the window's frame once showed (stripes, opaque).
Then the same with a box moving under the popup every 16 ms, to say what it
costs: the process's CPU time over `--seconds`, glass against no glass.

Each app runs in a Job Object of its own, which alone ends it; the probe sends
no input and captures only that app's windows. If a system dialog appears,
stop and report it; never click it. Windows only.
"""
import argparse
import ctypes
import pathlib
import subprocess
import sys
import tempfile
import time

import windows_input as win

APP = """
local STRIPES = ui.shader([[
fn fill(input:FillInput)->vec4<f32> {
 let on=step(4.,input.p.x-8.*floor(input.p.x/8.));
 return vec4(on,on,on,1.);
}
]])
local x = ui.state("x", 0)
%(moving)s
return ui.div {
    w_full = true, h_full = true, bg_shader = STRIPES,
    ui.div { absolute = true, left = x, top = 150, w = 40, h = 40, bg = "#ff0000" },
    ui.popover {
        open = ui.state("open", true), dismiss_on_backdrop = false, autofocus = false,
        anchor = ui.div { m = 40, w = 60, h = 28, bg = "#808080" },
        w = 240, h = 120, bg = "#111113a6", rounded = 8, backdrop_blur = %(blur)s,
    },
}
"""
MOVING = 'ui.every(16, function() x:set((x:get() + 1) % 240) end)'

kernel32 = ctypes.windll.kernel32
kernel32.CreateJobObjectW.restype = ctypes.c_void_p
kernel32.OpenProcess.restype = ctypes.c_void_p


class Job:
    """A Job Object whose closing ends what's in it, and only that."""

    class Limits(ctypes.Structure):
        _fields_ = [('per_process', ctypes.c_int64), ('per_job', ctypes.c_int64), ('flags', ctypes.c_uint32),
                    ('min_set', ctypes.c_size_t), ('max_set', ctypes.c_size_t), ('active', ctypes.c_uint32),
                    ('affinity', ctypes.c_size_t), ('priority', ctypes.c_uint32), ('scheduling', ctypes.c_uint32),
                    ('io', ctypes.c_uint64 * 6), ('process_memory', ctypes.c_size_t),
                    ('job_memory', ctypes.c_size_t), ('peak_process', ctypes.c_size_t), ('peak_job', ctypes.c_size_t)]

    def __init__(self, proc):
        self.handle = ctypes.c_void_p(kernel32.CreateJobObjectW(None, None))
        limits = Job.Limits(flags=0x2000)  # JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
        if not kernel32.SetInformationJobObject(self.handle, 9, ctypes.byref(limits), ctypes.sizeof(limits)):
            raise OSError('SetInformationJobObject')
        if not kernel32.AssignProcessToJobObject(self.handle, ctypes.c_void_p(int(proc._handle))):
            raise OSError('AssignProcessToJobObject')

    def close(self):
        kernel32.CloseHandle(self.handle)


def cpu_seconds(proc):
    times = [ctypes.c_uint64() for _ in range(4)]
    kernel32.GetProcessTimes(ctypes.c_void_p(int(proc._handle)), *[ctypes.byref(t) for t in times])
    return (times[2].value + times[3].value) / 1e7


def corner_ink(width, height, data):
    """The brightest pixel past the curve of the box's corners: the box found
    from the capture's middle row and column, the few pixels at each corner
    that an 8-point radius leaves out (nothing, in a transparent window)."""
    lit = lambda x, y: max(data[(y * width + x) * 4:(y * width + x) * 4 + 3])
    xs = [x for x in range(width) if lit(x, height // 2)]
    ys = [y for y in range(height) if lit(width // 2, y)]
    left, right, top, bottom = xs[0], xs[-1], ys[0], ys[-1]
    return max(
        lit(x, y)
        for across in range(3)
        for down in range(3 - across)
        for x in (left + across, right - across)
        for y in (top + down, bottom - down)
    )


def run(binary, stage, name, blur, moving, seconds):
    app_dir = stage / name
    app_dir.mkdir()
    (app_dir / 'app.luau').write_text(APP % {'blur': blur, 'moving': MOVING if moving else ''})
    log_path = stage / f'{name}.log'
    with open(log_path, 'w') as log:
        # Suspended until it's in its job: nothing of it runs outside.
        proc = subprocess.Popen([binary, str(app_dir)], cwd=stage, stdout=log, stderr=log,
                                creationflags=subprocess.CREATE_SUSPENDED)
    job = Job(proc)
    ctypes.windll.ntdll.NtResumeProcess(ctypes.c_void_p(int(proc._handle)))
    try:
        try:
            win.wait_window(proc)
        except RuntimeError as error:
            sys.exit(f'FAIL: {name}: {error}\n{log_path.read_text(errors="replace")[-2000:]}')
        deadline = time.monotonic() + 15
        while time.monotonic() < deadline and not win.popups_of(proc.pid):
            time.sleep(0.1)
        popups = win.popups_of(proc.pid)
        if not popups:
            sys.exit(f'FAIL: {name}: no popup window showed\n{log_path.read_text(errors="replace")[-2000:]}')
        time.sleep(2)
        if seconds:
            before = cpu_seconds(proc)
            time.sleep(seconds)
            return (cpu_seconds(proc) - before) / seconds
        width, height, data = win.capture_window(popups[0])
        row = height // 2
        levels = [data[(row * width + x) * 4 + 1] for x in range(width // 4, width * 3 // 4)]
        return min(levels), max(levels), width, height, corner_ink(width, height, data)
    finally:
        job.close()
        proc.wait()


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('binary')
    parser.add_argument('--seconds', type=float, default=10)
    args = parser.parse_args()
    binary = str(pathlib.Path(args.binary).resolve())
    with tempfile.TemporaryDirectory() as stage:
        stage = pathlib.Path(stage)
        low, high, width, height, corners = run(binary, stage, 'glass', 24, False, 0)
        print(f'glass: popup {width}x{height}, its middle row {low}..{high}, past its corners {corners}')
        sharp_low, sharp_high, *_ = run(binary, stage, 'clear', 0, False, 0)
        print(f'no blur: its middle row {sharp_low}..{sharp_high}')
        glass = run(binary, stage, 'glass-moving', 24, True, args.seconds)
        clear = run(binary, stage, 'clear-moving', 0, True, args.seconds)
        print(f'CPU, a box moving under it: glass {glass * 100:.1f}%, no blur {clear * 100:.1f}% of a core')
    if high - low > 12:
        sys.exit('FAIL: the glass shows the stripes sharp')
    if corners > 8:
        sys.exit('FAIL: the window shows past the rounded corners of the popup')
    if low - sharp_high < 20:
        sys.exit('FAIL: the glass holds nothing of its window: no brighter than its fill alone')
    print('PASS')


main()
