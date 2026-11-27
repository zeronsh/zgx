#!/usr/bin/env python3
"""A resting pointer over a streaming transcript, on X11: hover_walks_probe's
window on an owned Xvfb and Openbox, the real pointer moved over its
transcript while a reply streams in and the list follows its end. Each
frame that moves geometry looks for what is under the pointer once
(`refresh_hover_moved`), and the window's cursor after drawing
(`sync_cursor`) answers from that walk: no frame walks the scene twice.
Twice over: the real pointer alone, then with a driver's move into the
interface elsewhere (`zgx inspect`'s, an agent's), which the cursor follows
as hover does."""
import argparse
import json
import os
import pathlib
import re
import subprocess
import sys
import time

from platform_smoke import private_runtime, stop, wait_for_wm


def wait_for(log, pattern, seconds):
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        found = re.search(pattern, log.read_text(errors="replace"), re.M)
        if found:
            return found
        time.sleep(.05)
    raise AssertionError(f"no {pattern!r}: {log.read_text(errors='replace')[-3000:]}")


def run_probe(binary, output, runtime, driven):
    processes = []
    env = dict(os.environ, WINIT_X11_SCALE_FACTOR="1", XDG_RUNTIME_DIR=runtime)
    env.pop("WAYLAND_DISPLAY", None)
    env.pop("HOVER_WALKS_DRIVEN", None)
    if driven:
        env["HOVER_WALKS_DRIVEN"] = "1"
    try:
        xvfb = subprocess.Popen(["Xvfb", "-r", "-noreset", "-displayfd", "1", "-screen", "0", "1280x960x24", "-ac"],
                                env=env, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True)
        processes.append(xvfb)
        env["DISPLAY"] = ":" + xvfb.stdout.readline().strip()
        processes.append(subprocess.Popen(["openbox"], env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL))
        wait_for_wm(env)
        run = lambda *c: subprocess.run(c, env=env, capture_output=True, text=True).stdout.strip()
        log = output / f"hover-walks-{'driven' if driven else 'real'}.log"
        app = subprocess.Popen([str(binary)], env=env, stdout=log.open("w"), stderr=subprocess.STDOUT)
        processes.append(app)
        found = wait_for(log, r"^AT (\S+) (\S+)$", 30)
        x, y = float(found.group(1)), float(found.group(2))
        wait_for(log, r"^READY$", 30)
        window = None
        deadline = time.monotonic() + 20
        while window is None and time.monotonic() < deadline:
            for candidate in re.findall(r"0x[0-9a-f]+", run("xprop", "-root", "_NET_CLIENT_LIST")):
                if run("xprop", "-id", candidate, "_NET_WM_PID").endswith(f"= {app.pid}"):
                    window = str(int(candidate, 16))
            time.sleep(.05)
        assert window, "the probe's window did not show"
        info = run("xwininfo", "-id", window)
        origin = [int(re.search(rf"Absolute upper-left {a}:\s+(-?\d+)", info).group(1)) for a in "XY"]
        pointer = (origin[0] + round(x), origin[1] + round(y))
        # Moved onto it, then resting there.
        run("xdotool", "mousemove", str(pointer[0] - 3), str(pointer[1]))
        run("xdotool", "mousemove", str(pointer[0]), str(pointer[1]))
        found = wait_for(log, r"^WALKS frames=(\d+) walked=(\d+) twice=(\d+) most=(\d+)$", 60)
        frames, walked, twice, most = (int(found.group(n)) for n in range(1, 5))
        app.wait(timeout=20)
        return {"frames": frames, "walked": walked, "twice": twice, "most": most}
    finally:
        for process in reversed(processes):
            stop(process)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=pathlib.Path)
    parser.add_argument("--output", type=pathlib.Path, required=True)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    results = {}
    for mode, driven in (("real", False), ("driven", True)):
        with private_runtime("zgx-hover-walks-") as runtime:
            results[mode] = run_probe(args.binary.resolve(), args.output, runtime, driven)
    (args.output / "hover-walks.json").write_text(json.dumps(results, indent=2))
    print(json.dumps(results))
    for mode, result in results.items():
        # The reply streamed under the pointer: frames moved geometry, and
        # hover was looked for in them.
        if result["frames"] < 30 or result["walked"] < 10:
            sys.exit(f"{mode}: too little happened to judge: {result}")
        if result["twice"]:
            sys.exit(f"{mode}: {result['twice']} of {result['walked']} frames walked the scene more than "
                     f"once (at most {result['most']}): the cursor missed hover's walk")


if __name__ == "__main__":
    main()
