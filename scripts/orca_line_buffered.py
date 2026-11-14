#!/usr/bin/env python3
"""Orca (`/usr/bin/orca`, its arguments after this script's name) with its
debug file written a line at a time, for the probes that read what it says
from that file as it says it (`web_view_smoke.start_orca`).

Orca opens `--debug-file` with Python's default buffering, a block at a
time: what it said reached the file only as a block filled. With 8 KB
blocks (Python 3.12, Ubuntu 24.04) a Shift, which Orca echoes, now and then
filled one; with Python 3.14's 128 KB (26.04) nothing reached the file in a
probe's time, and the wait for "Screen reader on." ran out."""
import builtins
import runpy
import sys

_open = builtins.open


def _line_buffered(file, mode="r", *args, **kwargs):
    if "w" in mode and str(file) == _debug_file:
        kwargs["buffering"] = 1
    return _open(file, mode, *args, **kwargs)


_debug_file = next((a.split("=", 1)[1] for a in sys.argv[1:] if a.startswith("--debug-file=")), None)
builtins.open = _line_buffered
sys.argv = ["orca"] + sys.argv[1:]
runpy.run_path("/usr/bin/orca", run_name="__main__")
