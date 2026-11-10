#!/usr/bin/env python3
"""Native AT-SPI character geometry follows live owned XSETTINGS DPI changes."""
import argparse
import json
import os
import pathlib
import select
import signal
import subprocess
import sys
import tempfile
import time
from platform_smoke import release_mounts, stop, wait_for_wm
from live_dpi_pointer_smoke import XSettings


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=pathlib.Path)
    parser.add_argument("--output", type=pathlib.Path, required=True)
    parser.add_argument("--session", action="store_true", help=argparse.SUPPRESS)
    args = parser.parse_args()
    args.binary = args.binary.resolve()
    args.output = args.output.resolve()
    args.output.mkdir(parents=True, exist_ok=True)
    if not args.binary.is_file():
        parser.error("build the native scale example first")
    if not args.session:
        env = dict(os.environ, ZGX_PRIVATE_ATSPI="1")
        for key in ("AT_SPI_BUS_ADDRESS", "DBUS_SESSION_BUS_ADDRESS", "DISPLAY", "WAYLAND_DISPLAY", "WINIT_X11_SCALE_FACTOR", "SWAYSOCK", "I3SOCK"):
            env.pop(key, None)
        command = ["dbus-run-session", "--", sys.executable, __file__, str(args.binary), "--output", str(args.output), "--session"]
        raise SystemExit(subprocess.run(command, env=env).returncode)
    if os.environ.get("ZGX_PRIVATE_ATSPI") != "1":
        raise RuntimeError("private session required")
    xvfb = wm = launcher = app = settings = None
    result = dict(ok=False, samples=[], events=[])
    with tempfile.TemporaryDirectory(prefix="zgx-ax-scale-") as runtime:
        os.chmod(runtime, 0o700)
        try:
            with (args.output / "xvfb.log").open("w") as log:
                xvfb = subprocess.Popen(["Xvfb", "-r", "-noreset", "-displayfd", "1", "-screen", "0", "1100x900x24", "-ac"], stdout=subprocess.PIPE, stderr=log, text=True)
                if not select.select([xvfb.stdout], [], [], 30)[0]:
                    raise RuntimeError("owned Xvfb startup timed out")
                display = ":" + xvfb.stdout.readline().strip()
                if display == ":":
                    raise RuntimeError("owned Xvfb startup failed")
            os.environ.update(DISPLAY=display, XDG_RUNTIME_DIR=runtime, XDG_CONFIG_HOME=str(pathlib.Path(runtime, "config")), XDG_CACHE_HOME=str(pathlib.Path(runtime, "cache")), GSETTINGS_BACKEND="memory")
            os.environ.pop("WINIT_X11_SCALE_FACTOR", None)
            os.environ.pop("WAYLAND_DISPLAY", None)
            with (args.output / "openbox.log").open("w") as log:
                wm = subprocess.Popen(["openbox"], stdout=log, stderr=log)
            wait_for_wm(os.environ)
            settings = XSettings(display)  # starts at 96 DPI, before winit
            with (args.output / "a11y-bus.log").open("w") as log:
                launcher = subprocess.Popen(["/usr/libexec/at-spi-bus-launcher", "--launch-immediately", "--a11y=1", "--screen-reader=1"], stdout=log, stderr=log, start_new_session=True)
            import gi
            gi.require_version("Atspi", "2.0")
            from gi.repository import Atspi, Gio, GLib
            session = Gio.bus_get_sync(Gio.BusType.SESSION, None)

            def call(interface, method, parameters):
                return session.call_sync("org.a11y.Bus", "/org/a11y/bus", interface, method, parameters, None, Gio.DBusCallFlags.NO_AUTO_START, 2000, None)

            deadline = time.monotonic() + 8
            while True:
                try:
                    for key in ("IsEnabled", "ScreenReaderEnabled"):
                        call("org.freedesktop.DBus.Properties", "Set", GLib.Variant("(ssv)", ("org.a11y.Status", key, GLib.Variant("b", True))))
                    address = call("org.a11y.Bus", "GetAddress", None).unpack()[0]
                    break
                except GLib.Error:
                    if time.monotonic() >= deadline:
                        raise
                    time.sleep(.05)
            os.environ["AT_SPI_BUS_ADDRESS"] = address
            Atspi.init()
            context = GLib.MainContext.default()

            def wait_for(function, name, timeout=8):
                deadline = time.monotonic() + timeout
                while time.monotonic() < deadline:
                    while context.pending():
                        context.iteration(False)
                    try:
                        value = function()
                        if value:
                            return value
                    except GLib.Error:
                        pass
                    time.sleep(.03)
                raise AssertionError("timed out: " + name)

            def find(name):
                def visit(node, depth=0):
                    if node is None or depth > 30:
                        return None
                    node.clear_cache()
                    if node.get_name() == name:
                        return node
                    for i in range(node.get_child_count()):
                        found = visit(node.get_child_at_index(i), depth + 1)
                        if found is not None:
                            return found
                    return None
                return visit(Atspi.get_desktop(0))

            def record(event, *_):
                result["events"].append(dict(type=event.type, detail1=event.detail1, detail2=event.detail2))
            listener = Atspi.EventListener.new(record)
            for event in ("object:bounds-changed", "object:state-changed", "object:text-selection-changed"):
                assert listener.register(event)
            with (args.output / "application.log").open("w") as log:
                app = subprocess.Popen([str(args.binary), "--smoke-test"], stdout=log, stderr=log)
                target = wait_for(lambda: find("Scale target"), "native scale label")
                entry = wait_for(lambda: find("Scale field"), "native scale editor")

                def box(node):
                    node.clear_cache()
                    r = node.get_component_iface().get_extents(Atspi.CoordType.SCREEN)
                    return [r.x, r.y, r.width, r.height]

                def text_box(node, length):
                    r = node.get_text_iface().get_range_extents(0, length, Atspi.CoordType.SCREEN)
                    return [r.x, r.y, r.width, r.height]

                initial = box(target)
                initial_text = text_box(target, 5)
                result["initial"] = dict(bounds=initial, text=initial_text)
                assert initial_text[2] > 0 and initial_text[3] > 0
                for dpi, selected_range in ((192, (0, 1)), (120, (1, 3)), (96, (0, 2))):
                    ratio = dpi / 96
                    settings.set_dpi(dpi)
                    current = wait_for(lambda: r if abs((r := box(target))[2] - initial[2] * ratio) <= 1 else None, "native bounds reflect actual live DPI")
                    # The main node may be current while old TextRun properties
                    # still use the prior scale. Compare range geometry itself.
                    r = text_box(target, 5)
                    sample = dict(dpi=dpi, ratio=ratio, bounds=current, text=r)
                    result["samples"].append(sample)
                    for axis in (2, 3):
                        assert abs(r[axis] - initial_text[axis] * ratio) <= 2, sample
                    hit = target.get_component_iface().get_accessible_at_point(current[0] + 2, current[1] + 2, Atspi.CoordType.SCREEN)
                    assert hit is not None, "native hit query remains live"
                    if dpi != 192:
                        entry.clear_cache()
                        assert entry.get_state_set().contains(Atspi.StateType.FOCUSED), "scale preserves native focus"
                    assert entry.get_component_iface().grab_focus()
                    def focused():
                        entry.clear_cache()
                        return entry.get_state_set().contains(Atspi.StateType.FOCUSED)
                    wait_for(focused, "native focus after scale")
                    text = entry.get_text_iface()
                    if text.get_n_selections():
                        assert text.remove_selection(0)
                    assert text.add_selection(*selected_range)
                    def selected():
                        if not text.get_n_selections():
                            return None
                        selection = text.get_selection(0)
                        return selection if (selection.start_offset, selection.end_offset) == selected_range else None
                    wait_for(selected, "fresh native selection after scale")
                    assert text.get_text(0, -1) == "Ada"
                wait_for(lambda: any(e["type"].startswith("object:bounds-changed") for e in result["events"]), "scale bounds events")
                result["ok"] = True
        except Exception as error:
            result["error"] = repr(error)
            raise
        finally:
            (args.output / "results.json").write_text(json.dumps(result, indent=2) + "\n")
            stop(app)
            if settings is not None:
                settings.close()
            stop(wm)
            if launcher is not None:
                try:
                    os.killpg(launcher.pid, signal.SIGTERM)
                except ProcessLookupError:
                    pass
                stop(launcher)
            stop(xvfb)
            release_mounts(runtime)


if __name__ == "__main__":
    main()
