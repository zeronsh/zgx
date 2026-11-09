#!/usr/bin/env python3
"""What a window holds at rest, by category: runs `rest_memory` (macOS, a real
window, its environment clean), reads `footprint -p` from outside at each point
it prints, and tables the categories. A run whose window stopped presenting,
covered by another, says so: what it holds isn't what a window at rest does.

    cargo build --release -p zgx-desktop --example rest_memory --features mimalloc
    python3 scripts/rest_memory.py target/release/examples/rest_memory [--output DIR] [NAME=VALUE ...]

CFLAGS_aarch64_apple_darwin="-DMI_ARENA_SLICE_SHIFT=14 -DMI_ENABLE_LARGE_PAGES=0"
as it builds keeps a few MB less at rest: build with it to compare.
"""
import argparse
import os
import pathlib
import re
import subprocess
import tempfile

UNITS = {"B": 1 / 1048576, "KB": 1 / 1024, "MB": 1, "GB": 1024}
LINE = re.compile(r"\s*([\d.]+) (B|KB|MB|GB)\s+[\d.]+ (?:B|KB|MB|GB)\s+[\d.]+ (?:B|KB|MB|GB)\s+\d+\s+(.+)$")


def categories(text):
    found = {}
    for line in text.splitlines():
        match = LINE.match(line)
        if match and match[3].strip() != "TOTAL":
            found[match[3].strip()] = float(match[1]) * UNITS[match[2]]
    return found


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("binary", type=pathlib.Path)
    parser.add_argument("--output", type=pathlib.Path)
    parser.add_argument("env", nargs="*", help="NAME=VALUE for the example")
    args = parser.parse_args()
    output = args.output or pathlib.Path(tempfile.mkdtemp(prefix="rest-memory-"))
    output.mkdir(parents=True, exist_ok=True)
    env = {name: os.environ[name] for name in ("HOME", "USER", "LOGNAME") if name in os.environ}
    env["PATH"] = "/usr/bin:/bin:/usr/sbin:/sbin"
    env.update(item.split("=", 1) for item in args.env)
    app = subprocess.Popen([str(args.binary)], env=env, stdout=subprocess.PIPE, text=True, bufsize=1)
    points, covered, report = {}, [], (output / "report.txt").open("w")
    for line in app.stdout:
        report.write(line)
        report.flush()
        if "| host | presented false" in line:
            covered.append(line.split(" | ")[0].removeprefix("MEM "))
        point = re.match(r"POINT (.*?): footprint ([\d.]+)", line)
        if point:
            read = subprocess.run(["footprint", "-p", str(app.pid)], capture_output=True, text=True, timeout=120)
            (output / (point[1].replace(" ", "_") + ".txt")).write_text(read.stdout)
            points[point[1]] = categories(read.stdout)
    app.wait()
    names = sorted({name for values in points.values() for name in values},
                   key=lambda name: -max(values.get(name, 0) for values in points.values()))
    print(f"{'MiB dirty':44}" + "".join(f"{point[:10]:>11}" for point in points))
    for name in names:
        if max(values.get(name, 0) for values in points.values()) >= 0.5:
            print(f"{name[:43]:44}" + "".join(f"{values.get(name, 0):11.1f}" for values in points.values()))
    if covered:
        print("NOT AT REST: the window wasn't presenting at: " + ", ".join(covered))
    print(f"reports in {output}")


if __name__ == "__main__":
    main()
