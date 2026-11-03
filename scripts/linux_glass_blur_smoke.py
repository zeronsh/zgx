#!/usr/bin/env python3
"""A window with glass asks a Wayland compositor to blur behind it
(winit's `set_blur`, `ext-background-effect-v1` or KWin's
`org_kde_kwin_blur`), and only where the compositor has a protocol for it:
glass_counter under a headless sway with `WAYLAND_DEBUG=1`, its own
requests read from its log.

Checks that:

- the window is configured and drawn, with no protocol error;
- where the compositor advertises `ext_background_effect_manager_v1`, the
  window's surface asks for its background effect and a blur region; where
  only KWin's blur is advertised, it asks KWin's; where neither is (sway,
  GNOME, Weston), it asks nothing, and nothing fails;
- on X11 (Xvfb) the window still runs: winit's X11 `set_blur` sends
  nothing."""
import argparse
import json
import os
import pathlib
import re
import select
import subprocess
import sys
import tempfile
import time

sys.path.insert(0, str(pathlib.Path(__file__).parent))
from platform_smoke import release_mounts, stop  # noqa: E402
from sway_host import sway_config  # noqa: E402


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binaries", type=pathlib.Path, help="the zgx-desktop examples' directory")
    parser.add_argument("--output", type=pathlib.Path, required=True)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    processes, checks = [], {}

    def launch(name, command, env):
        log = args.output / (name + ".log")
        processes.append(subprocess.Popen(list(map(str, command)), env=env, stdout=log.open("w"),
                                          stderr=subprocess.STDOUT))
        return processes[-1], log

    def until(what, test, seconds=20):
        deadline = time.monotonic() + seconds
        while True:
            value = test()
            if value:
                return value
            if time.monotonic() > deadline:
                raise AssertionError(f"timed out waiting for {what}: {checks}")
            time.sleep(.1)

    try:
        with tempfile.TemporaryDirectory(prefix="zgx-glass-blur-") as private:
            os.chmod(private, 0o700)
            env = dict(os.environ, XDG_RUNTIME_DIR=private, WLR_BACKENDS="headless", WLR_RENDERER="pixman",
                       WLR_LIBINPUT_NO_DEVICES="1", NO_AT_BRIDGE="1")
            for key in ("DISPLAY", "WAYLAND_DISPLAY", "DBUS_SESSION_BUS_ADDRESS"):
                env.pop(key, None)
            config = pathlib.Path(private, "sway.conf")
            sway_config(config, "output * mode 1280x800\ndefault_border none\n")
            launch("sway", ["sway", "--unsupported-gpu", "-c", config], env)
            sockets = lambda: [p for p in pathlib.Path(private).glob("wayland-*") if p.suffix != ".lock"]
            env["WAYLAND_DISPLAY"] = until("sway's socket", sockets, 10)[0].name
            app, log = launch("glass_counter", [args.binaries / "glass_counter"],
                              dict(env, WAYLAND_DEBUG="1"))
            text = lambda: log.read_text(errors="replace")
            # Configured, and a buffer on its surface: the window is up.
            until("the window configured and drawn",
                  lambda: "ack_configure(" in text() and re.search(r"-> wl_surface@\d+\.attach\(", text()))
            time.sleep(1.5)
            if app.poll() is not None:
                raise AssertionError(f"glass_counter exited with {app.returncode}; see glass_counter.log")
            log_text = text()
            if "wl_display@1.error(" in log_text:
                raise AssertionError("a protocol error; see glass_counter.log")
            advertised = set(re.findall(r'wl_registry@\d+\.global\(\d+, "([a-z0-9_]+)", \d+\)', log_text))
            standard = "ext_background_effect_manager_v1" in advertised
            kwin = "org_kde_kwin_blur_manager" in advertised
            asked_standard = bool(re.search(r"-> ext_background_effect_manager_v1@\d+\.get_background_effect\(",
                                            log_text)) and bool(
                re.search(r"-> ext_background_effect_surface_v1@\d+\.set_blur_region\(", log_text))
            asked_kwin = bool(re.search(r"-> org_kde_kwin_blur_manager@\d+\.create\(", log_text))
            checks["advertised"] = {"ext-background-effect": standard, "kwin blur": kwin}
            checks["asked"] = {"ext-background-effect": asked_standard, "kwin blur": asked_kwin}
            if standard and not asked_standard:
                raise AssertionError(f"the compositor has ext-background-effect but no blur was asked: {checks}")
            if not standard and kwin and not asked_kwin:
                raise AssertionError(f"the compositor has KWin's blur but none was asked: {checks}")
            if not standard and asked_standard or not kwin and asked_kwin:
                raise AssertionError(f"a blur asked of a protocol the compositor never advertised: {checks}")
            if standard and asked_kwin:
                raise AssertionError(f"KWin's blur asked where the standard protocol is there: {checks}")
            checks["wayland"] = "blur asked" if asked_standard or asked_kwin else "nothing to ask, nothing asked"
            stop(app)
            processes.remove(app)
            for process in processes:
                stop(process)
            processes.clear()

            # X11: winit's `set_blur` is a no-op there; the window runs.
            # A display of its own, the first free one (`-displayfd`): probes
            # run side by side.
            with (args.output / "xvfb.log").open("w") as log:
                xvfb = subprocess.Popen(["Xvfb", "-r", "-noreset", "-displayfd", "1", "-screen", "0", "1280x800x24",
                                         "-ac"], stdout=subprocess.PIPE, stderr=log, text=True)
            processes.append(xvfb)
            if not select.select([xvfb.stdout], [], [], 30)[0]:
                raise AssertionError("Xvfb never started; see xvfb.log")
            display = xvfb.stdout.readline().strip()
            if not display:
                raise AssertionError("Xvfb failed; see xvfb.log")
            x_env = dict(os.environ, XDG_RUNTIME_DIR=private, DISPLAY=":" + display, NO_AT_BRIDGE="1")
            for key in ("WAYLAND_DISPLAY", "DBUS_SESSION_BUS_ADDRESS"):
                x_env.pop(key, None)
            app, x_log = launch("glass_counter-x11", [args.binaries / "glass_counter"], x_env)
            found = lambda: subprocess.run(["xdotool", "search", "--onlyvisible", "--pid", str(app.pid)], env=x_env,
                                           capture_output=True, text=True).stdout.split()
            until("the X11 window", found)
            time.sleep(1.5)
            if app.poll() is not None or "panicked" in x_log.read_text(errors="replace"):
                raise AssertionError("glass_counter failed on X11; see glass_counter-x11.log")
            checks["x11"] = "runs"
            for process in processes:
                stop(process)
            processes.clear()
            # zgx's settings read starts the document portal, which mounts
            # itself in the private runtime directory.
            release_mounts(private)
        result = {"result": "pass", "checks": checks}
        print("PASS: " + json.dumps(checks))
    except Exception as error:
        result = {"result": "fail", "error": str(error), "checks": checks}
        raise
    finally:
        for process in processes:
            stop(process)
        (args.output / "result.json").write_text(json.dumps(result if "result" in locals() else {}, indent=2))


if __name__ == "__main__":
    main()
