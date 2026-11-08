#!/usr/bin/env python3
"""What the UI thread spends its time on: sample a running iOS simulator app's
main thread and bucket its time by what it is doing, innermost match first.
Simulator results: real ARM CPU, but no phone's speed, and the Mac's GPU."""
import re
import subprocess
import sys
import time
from collections import Counter

# Innermost frame first: the first rule to match along the stack takes the sample.
BUCKETS = [
    ("Luau VM (interpreter)", r"luau_|luaV_|luaD_|luaH_|luaG_|luaC_|luaL_|lua_|luaM_|luaU_|luaF_|luaT_|luaS_|Luau::"),
    ("Luau compile", r"luau_compile|Luau::Compile|Luau::Parser|Luau::compile|luaU_undump"),
    ("mlua bridge", r"mlua::|zgx_mlua"),
    ("text shaping", r"harfrust|cosmic_text::shape|ShapeLine|ShapeBuffer|shape_run|rustybuzz"),
    ("text layout other", r"cosmic_text"),
    ("glyph raster", r"swash|skrifa|zeno|prepare_glyph|glyph_image|fontdue"),
    ("markdown", r"markdown|pulldown"),
    ("layout", r"taffy|layout::|compute_layout|zgx::layout"),
    ("image decode", r"zune|png::|jpeg|image::|gif::|resvg|usvg"),
    ("io", r"\bread\b|\bopen\b|__open|__read|fopen|stat\b|std::fs"),
    ("gpu encode", r"wgpu|metal|MTL|Metal|naga|zgx_gpu"),
    ("system compositing", r"CA::|QuartzCore|CAMetal|CATransaction"),
    ("uikit", r"UIKit|UIApplication|_UI|UIView"),
    ("scene and reactive", r"zgx::scene|zgx::reactive|zgx::compose|zgx::widgets|zgx::input|zgx_script|zgx::"),
    ("waiting", r"mach_msg|__psynch|kevent|__workq|semaphore"),
]


def categorize(frames):
    for frame in reversed(frames):
        for name, pattern in BUCKETS:
            if re.search(pattern, frame):
                return name
    return "other"


def main_thread_buckets(text):
    start = text.find("Call graph:")
    lines = text[start:].splitlines()[1:]
    counts = Counter()
    stack = []
    in_main = False
    for line in lines:
        match = re.match(r"^([\s+!:|]*)(\d+)\s+(.*)$", line)
        if not match:
            if line.strip().startswith("Total number"):
                break
            continue
        depth = len(match.group(1))
        count, name = int(match.group(2)), match.group(3)
        if name.startswith("Thread_") or "Thread_" in name[:20]:
            in_main = "main-thread" in name
            stack = []
            continue
        if not in_main:
            continue
        while stack and stack[-1][0] >= depth:
            stack.pop()
        stack.append((depth, name, count))
        children = None
        # A frame's own samples are its count less its children's, settled when
        # each child arrives.
        if len(stack) >= 2:
            parent = stack[-2]
            stack[-2] = (parent[0], parent[1], parent[2] - count)
            counts[categorize([f[1] for f in stack[:-1]])] -= count
        counts[categorize([f[1] for f in stack])] += count
    return counts


def sample(pid, seconds):
    path = f"/tmp/zgx-threads-{pid}.txt"
    subprocess.run(["/usr/bin/sample", str(pid), str(seconds), "1", "-mayDie", "-file", path], capture_output=True)
    with open(path, errors="replace") as f:
        return f.read()


def main():
    pid, seconds = int(sys.argv[1]), float(sys.argv[2])
    counts = main_thread_buckets(sample(pid, seconds))
    waiting = counts.pop("waiting", 0)
    busy = sum(v for v in counts.values() if v > 0)
    total = busy + waiting
    print(f"main thread busy {100 * busy / max(total, 1):.1f}% of {total} samples")
    for name, value in counts.most_common():
        if value > 0:
            print(f"  {name:24}{100 * value / max(busy, 1):6.1f}% of busy")


if __name__ == "__main__":
    main()
