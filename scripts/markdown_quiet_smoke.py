#!/usr/bin/env python3
"""A real window releases a quiet Markdown parser without another frame.

The script observes its cached answer through a weak table: the parser must
keep it during a pause, then let it go after its minute of quiet.
"""
import argparse
import json
import os
import pathlib
import re
import subprocess
import tempfile
import time

from inspect_memory_smoke import ask

APP = """
local weak = setmetatable({}, { __mode = "v" })
do
    local text = string.rep("A paragraph.\\n\\n", 128)
    ui.markdown(text, { key = "reply" })
    weak[1] = ui.markdown(text .. "Last paragraph.", { key = "reply" })
end
local held = ui.shared("probe.held", true)
ui.every(1000, function() held:set(weak[1] ~= nil) end)
return ui.column { ui.text { "Quiet stream" } }
"""


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("probe", type=pathlib.Path, help="zgx-run")
    parser.add_argument("--output", type=pathlib.Path, required=True)
    parser.add_argument("--native", action="store_true")
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    # Unix socket paths must fit on macOS even when --output is a long path.
    work = pathlib.Path(tempfile.mkdtemp(prefix="zgx-quiet-", dir="/tmp"))
    app_dir = work / "app"
    app_dir.mkdir()
    (app_dir / "app.luau").write_text(APP)
    env = dict(os.environ, HOME=str(work), TMPDIR=str(work), ZGX_INSPECT_SOCKET="1", ZGX_ONTOP="1", ZGX_CHECK="0")
    xvfb = app = None
    log = (args.output / "app.log").open("w")
    try:
        if not args.native:
            xvfb = subprocess.Popen(
                ["Xvfb", "-r", "-noreset", "-displayfd", "1", "-screen", "0", "1024x768x24", "-ac"],
                stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True,
            )
            env["DISPLAY"] = ":" + xvfb.stdout.readline().strip()
            env.pop("WAYLAND_DISPLAY", None)
        app = subprocess.Popen([str(args.probe.resolve()), str(app_dir)], env=env,
                               stdout=log, stderr=log)
        path = work / f"zgx-inspect-{os.getuid()}" / f"{app.pid}.sock"
        deadline = time.monotonic() + 60
        while not path.exists():
            assert app.poll() is None, "window exited before serving"
            assert time.monotonic() < deadline, "window did not serve"
            time.sleep(0.25)
        time.sleep(20)
        warm = ask(path, {"cmd": "state", "name": "probe.held"})
        assert warm.get("ok") and re.search(r"\btrue\b", warm.get("text", "")), warm
        ask(path, {"cmd": "mark", "name": "quiet"})
        time.sleep(55)
        cold = ask(path, {"cmd": "state", "name": "probe.held"})
        frames = ask(path, {"cmd": "frames", "since": "quiet", "all": True})
        (args.output / "answers.json").write_text(json.dumps({"warm": warm, "cold": cold, "frames": frames}, indent=2))
        assert cold.get("ok") and re.search(r"\bfalse\b", cold.get("text", "")), cold
        assert frames.get("ok") and not frames.get("data"), frames
        print("PASS: a stream stays during a pause and expires without another frame")
    finally:
        if app and app.poll() is None:
            app.kill()
            app.wait(timeout=10)
        if xvfb:
            xvfb.terminate()
            xvfb.wait(timeout=10)
        log.close()


if __name__ == "__main__":
    main()
