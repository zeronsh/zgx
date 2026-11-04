#!/usr/bin/env python3
"""Baked loops in a zoomed window on macOS, in a real window of zgx-run.

    python3 scripts/macos_zoomed_loops_probe.py [--target-dir DIR]

A Luau app zooms its window (100%, 110%, 90%) and shows, in a row with nothing
between them, a shader's loop as a status ring declares it and a still box the
window's frames draw. The loop's Core Animation layer must lie where the frames
show that row: read from window captures, the loop's box has the still box's
top and height and ends where it starts.

Needs Screen Recording permission for the shell running it, which it only
checks. Launched from /tmp with a clean environment. If a system dialog
appears, stop and report it; never click it. macOS only.
"""
import argparse
import os
import pathlib
import platform
import shutil
import subprocess
import sys
import tempfile
import time

ROOT = pathlib.Path(__file__).resolve().parents[1]
APP = """
ui.window.zoom:set(%(zoom)s)
local WHITE = ui.shader([[
fn fill(input:FillInput)->vec4<f32> {
 let level=.8+.2*fract(input.time);
 return vec4(level,level,level,1.);
}
]])
return ui.column {
    p = 90, bg = "#000000", w_full = true, h_full = true,
    ui.row {
        ui.div {
            w = 40, h = 40, shrink = 0, bg_shader = WHITE,
            on_mount = function(el)
                anime.animate(el, { shaderTime = { 0, 3.9 }, duration = 3900, loop = true,
                    loopDelay = 900, ease = "linear", frameRate = 30 })
            end,
        },
        ui.div { w = 40, h = 40, shrink = 0, bg = "#00ff00" },
    },
}
"""


def boxes(png):
    """The loop's box and the still box's in a capture, as left, top, right,
    bottom pixels (`sips` reads no pixels: a BMP does)."""
    bmp = png.with_suffix('.bmp')
    subprocess.run(['sips', '-s', 'format', 'bmp', str(png), '--out', str(bmp)], capture_output=True, check=True)
    data = bmp.read_bytes()
    start = int.from_bytes(data[10:14], 'little')
    width = int.from_bytes(data[18:22], 'little')
    height = int.from_bytes(data[22:26], 'little', signed=True)
    depth = int.from_bytes(data[28:30], 'little') // 8
    stride = (width * depth + 3) // 4 * 4
    white, green = None, None

    def grow(box, x, y):
        return (min(box[0], x), min(box[1], y), max(box[2], x + 1), max(box[3], y + 1)) if box else (x, y, x + 1, y + 1)

    def pixel(x, row):
        return data[start + row * stride + x * depth:start + row * stride + x * depth + 3]

    def row_of(y):
        return abs(height) - 1 - y if height > 0 else y

    for y in range(abs(height)):
        for x in range(width):
            blue, grn, red = pixel(x, row_of(y))
            if grn > 200 and red < 60 and blue < 60:
                green = grow(green, x, y)
    if not green:
        return None, None
    # Near the still box's rows alone: the title bar's text is as white, and a
    # misplaced loop is still within a box's height of them.
    tall = green[3] - green[1]
    for y in range(max(green[1] - tall, 0), min(green[3] + tall, abs(height))):
        for x in range(width):
            blue, grn, red = pixel(x, row_of(y))
            if red > 120 and grn > 120 and blue > 120:
                white = grow(white, x, y)
    return white, green


def main():
    if platform.system() != 'Darwin':
        print('macOS only')
        return
    parser = argparse.ArgumentParser()
    parser.add_argument('--target-dir', default=os.environ.get('CARGO_TARGET_DIR', str(ROOT / 'target')))
    args = parser.parse_args()
    env = dict(os.environ, CARGO_TARGET_DIR=args.target_dir)
    subprocess.run(['cargo', 'build', '--release', '--locked', '-p', 'zgx-run'], cwd=ROOT, env=env, check=True)
    failed = []
    with tempfile.TemporaryDirectory() as stage:
        stage = pathlib.Path(stage)
        binary = stage / 'zgx-run'
        shutil.copy2(pathlib.Path(args.target_dir) / 'release/zgx-run', binary)
        pixels = stage / 'window_pixels'
        subprocess.run(['swiftc', '-O', str(ROOT / 'scripts/window_pixels.swift'), '-o', str(pixels)], check=True)
        clean = {'HOME': os.environ['HOME'], 'PATH': '/usr/bin:/bin', 'ZGX_PRESENT_LOG': '1'}
        if 'TMPDIR' in os.environ:
            clean['TMPDIR'] = os.environ['TMPDIR']
        for zoom in ['1', '1.1', '0.9']:
            app_dir = stage / f'app{zoom}'
            app_dir.mkdir()
            (app_dir / 'app.luau').write_text(APP % {'zoom': zoom})
            log_path = stage / f'log{zoom}'
            with open(log_path, 'w') as log:
                app = subprocess.Popen([str(binary), str(app_dir)], cwd='/tmp', env=clean, stdout=log, stderr=log)
            try:
                time.sleep(5)
                shot = stage / f'shot{zoom}.png'
                done = subprocess.run([str(pixels), 'capture', str(app.pid), str(shot)], capture_output=True, text=True)
                if done.returncode != 0:
                    sys.exit(f'capture failed: {done.stdout} {done.stderr}')
                white, green = boxes(shot)
                baked = 'loop-bake' in log_path.read_text(errors='replace')
            finally:
                app.terminate()
                app.wait()
            print(f'zoom {zoom}: loop {white}, still {green}, baked {baked}')
            if not baked:
                failed.append(f'{zoom}: no loop was baked')
            elif not white or not green:
                failed.append(f'{zoom}: a box did not show')
            elif (abs(white[1] - green[1]) > 1 or abs(white[3] - green[3]) > 1 or abs(white[2] - green[0]) > 1
                    or abs((white[2] - white[0]) - (green[2] - green[0])) > 1):
                failed.append(f'{zoom}: the loop is not beside the still box')
    if failed:
        sys.exit('FAIL: ' + '; '.join(failed))
    print('PASS')


main()
