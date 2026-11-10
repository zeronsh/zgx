#!/usr/bin/env python3
"""CPU-only trims cannot keep a real window's grown glyph atlas warm.

Run with the rest_memory example, on a private X display by default;
--native uses the current desktop (macOS too).
"""
import argparse
import json
import os
import pathlib
import re
import subprocess
import tempfile


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("probe", type=pathlib.Path)
    parser.add_argument("--output", type=pathlib.Path, required=True)
    parser.add_argument("--native", action="store_true")
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    work = pathlib.Path(tempfile.mkdtemp(dir=args.output))
    env = dict(os.environ, HOME=str(work), TMPDIR=str(work), IDLE_REQUESTS="1", SCROLL="4")
    xvfb = None
    try:
        if not args.native:
            xvfb = subprocess.Popen(
                ["Xvfb", "-r", "-noreset", "-displayfd", "1", "-screen", "0", "1200x1000x24", "-ac"],
                stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True,
            )
            env["DISPLAY"] = ":" + xvfb.stdout.readline().strip()
            env.pop("WAYLAND_DISPLAY", None)
        result = subprocess.run([str(args.probe.resolve())], env=env, capture_output=True,
                                text=True, timeout=150)
        (args.output / "renderer-rest.log").write_text(result.stdout + result.stderr)
        assert result.returncode == 0, result.stderr
        atlas = {}
        for line in result.stdout.splitlines():
            match = re.match(r"MEM (.*?) \| gpu \| glyph atlas \| .* \| ([\d.]+) MB", line)
            if match:
                atlas[match[1]] = float(match[2])
        warm = max(atlas.get("mounted", 0), atlas.get("scrolling", 0), atlas.get("idle 5 s", 0))
        (args.output / "atlas.json").write_text(json.dumps(atlas, indent=2))
        assert warm > 2, f"the fixture did not grow its atlas: {atlas}"
        assert atlas.get("idle 35 s", warm) < warm / 2, f"CPU trims postponed renderer rest: {atlas}"
        print(f"PASS: repeated CPU-only trims release the atlas: {warm} -> {atlas['idle 35 s']} MB")
    finally:
        if xvfb:
            xvfb.terminate()
            xvfb.wait(timeout=10)


if __name__ == "__main__":
    main()
