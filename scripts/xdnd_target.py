#!/usr/bin/env python3
"""A test-owned drop target for drag probes: a GTK 3 window that takes drops
(GTK's own XDND on X11, its `wl_data_device` on Wayland with
`GDK_BACKEND=wayland`) and prints what each one carried.

    xdnd_target.py TITLE X Y [--refuse] [--image]

Each drop prints `RECEIVED <target> <text>` with the format it asked for;
with `--image` it asks for PNG first, and prints `RECEIVED image/png <n>
bytes <first four, in hex>` for one.
With `--refuse` it answers every position "not here", so a drag released
over it is not dropped; it prints `MOTION` as positions arrive."""
import argparse
import sys

import gi

gi.require_version("Gtk", "3.0")
gi.require_version("Gdk", "3.0")
from gi.repository import Gdk, GLib, Gtk  # noqa: E402

# The formats asked for, most preferred first.
FORMATS = ["text/uri-list", "UTF8_STRING", "text/plain;charset=utf-8", "text/plain"]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("title")
    parser.add_argument("x", type=int)
    parser.add_argument("y", type=int)
    parser.add_argument("--refuse", action="store_true")
    # A slow target: it finishes with a drop this long after taking its data.
    parser.add_argument("--finish-delay", type=float, default=0.)
    parser.add_argument("--image", action="store_true", help="ask for PNG first")
    args = parser.parse_args()
    formats = (["image/png"] if args.image else []) + FORMATS

    window = Gtk.Window(title=args.title)
    window.set_default_size(400, 200)
    window.move(args.x, args.y)
    window.connect("destroy", Gtk.main_quit)
    area = Gtk.EventBox()
    area.add(Gtk.Label(label=args.title))
    window.add(area)

    targets = [Gtk.TargetEntry.new(name, 0, index) for index, name in enumerate(formats)]
    # Motion and drops answered here, not by GTK's defaults.
    area.drag_dest_set(0, targets, Gdk.DragAction.COPY)

    def offered(context):
        names = [str(atom) for atom in context.list_targets()]
        return next((name for name in formats if name in names), None)

    def motion(widget, context, x, y, time):
        if args.refuse:
            print("MOTION", flush=True)
            Gdk.drag_status(context, 0, time)
            return True
        Gdk.drag_status(context, Gdk.DragAction.COPY if offered(context) else 0, time)
        return True

    def drop(widget, context, x, y, time):
        name = offered(context)
        if args.refuse or name is None:
            return False
        widget.drag_get_data(context, Gdk.Atom.intern(name, False), time)
        return True

    def received(widget, context, x, y, data, info, time):
        if formats[info].startswith("image/"):
            raw = data.get_data() if data.get_length() >= 0 else b""
            print("RECEIVED", formats[info], len(raw), "bytes", raw[:4].hex(), flush=True)
        else:
            text = data.get_data().decode("utf-8", "replace") if data.get_length() >= 0 else ""
            print("RECEIVED", formats[info], repr(text), flush=True)

        def finish():
            Gtk.drag_finish(context, data.get_length() >= 0, False, time)
            return False

        if args.finish_delay > 0:
            GLib.timeout_add(int(args.finish_delay * 1000), finish)
        else:
            finish()

    area.connect("drag-motion", motion)
    area.connect("drag-drop", drop)
    area.connect("drag-data-received", received)
    window.show_all()
    print("READY", flush=True)
    Gtk.main()


if __name__ == "__main__":
    sys.exit(main())
