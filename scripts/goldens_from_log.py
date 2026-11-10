#!/usr/bin/env python3
"""Write the reference images a CI log carries (zgx-3d's tests/reference.rs
prints `ZGX-GOLDEN <family>/<name>.png <base64 PNG>` where a GPU is required
and a golden is missing or differs) into crates/zgx-3d/tests/reference/.

    gh run view RUN --log | python3 scripts/goldens_from_log.py
"""
import base64
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent / "crates" / "zgx-3d" / "tests" / "reference"
LINE = re.compile(r"ZGX-GOLDEN ([a-z0-9-]+/[a-z0-9_]+\.png) ([A-Za-z0-9+/=]+)")


def main():
    written = 0
    for line in sys.stdin:
        found = LINE.search(line)
        if not found:
            continue
        path = ROOT / found.group(1)
        data = base64.b64decode(found.group(2))
        if not data.startswith(b"\x89PNG\r\n\x1a\n"):
            raise SystemExit(f"{found.group(1)}: not a PNG")
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(data)
        print(f"wrote {path.relative_to(ROOT.parent.parent.parent.parent)} ({len(data)} bytes)")
        written += 1
    if not written:
        raise SystemExit("no ZGX-GOLDEN lines in the input")


if __name__ == "__main__":
    main()
