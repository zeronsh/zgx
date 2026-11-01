#!/usr/bin/env python3
"""A web page in a zgx window on Linux, in an owned session: WebKitGTK's
page, drawn offscreen and shown as an image (`web_view_probe`, built with
`--features webkitgtk`).

Checks that the page shows, clipped to its box's rounded corners, with zgx
content drawn over it; that clicks land where they are on the page, typing
reaches its field and the wheel scrolls it; that a `<select>`'s popup (a zgx
menu) chooses from the keyboard; that keys go to the page first and those it
leaves alone reach the app's shortcuts, while a reserved one never reaches
the page; that what the page copies reaches other clients and what they
copy pastes into the page; and that a script evaluates and the page's
messages carry their origin.

`--wayland`: the app on Wayland (Sway on an X11 output, no DISPLAY for the
app; the clipboard through wl-clipboard). `--ime`: Pinyin composed into the
page's field instead (IBus over XIM on X11; Fcitx5 over text-input-v3 in
Sway with `--wayland`), checked as the page's composition events and value.
`--slow-replies`: WebKit's answers 800 ms late, as on a loaded machine: on
each key, after Ctrl is up (a shortcut the page leaves alone still reaches
the app with the modifiers it was pressed with); with `--ime`, where the
caret is, after IBus's candidates showed (they still go under it).
`--transparent`: a transparent page with no background of its own shows
its box's yellow through it. `--orca` (X11): over a private AT-SPI bus, the
page's own accessible tree is a child of its box (an internal frame) and
has the box for its parent, and Orca reads the page's heading from browse
mode and names its first field as Tab moves focus there from the app's own
button.
`--media` (X11): a page served from localhost plays a VP8 WebM (GStreamer
made it), and after a click an `AudioContext` runs. `--inspector`: on a
page large enough for WebKit to dock Web Inspector in, Ctrl+I opens it in a
window of its own, no frame of it docked shows in the page, and its
console, typed into, reaches the page.
"""
import argparse
import functools
import http.server
import json
import os
import pathlib
import re
import select
import shutil
import signal
import subprocess
import sys
import threading
import time

from platform_smoke import private_runtime, stop, use_ibus_engine, wait_for_ibus_xim, wait_for_wm
from sway_host import wait_for_sway_host, sway_config

# The probe's geometry (window points; probes run at scale 1).
PAGE = (40, 40, 600, 400)
COLOURS = {
    "page": ((340, 340), (0, 0, 255)),
    "target": ((70, 150), (0, 255, 0)),
    "square over the page": ((620, 420), (255, 0, 0)),
    "clipped corner": ((42, 42), (0x20, 0x20, 0x20)),
}


def near(found, wanted, tolerance=24):
    return all(abs(a - b) <= tolerance for a, b in zip(found[:3], wanted))


class Session:
    def __init__(self, output, env, window, origin=(0, 0), wayland=None):
        self.output, self.env, self.window, self.origin = output, env, window, origin
        self.wayland = wayland
        self.log = output / "application.log"

    def xdo(self, *parts):
        subprocess.run(["xdotool", *map(str, parts)], env=self.env, check=True)

    def click(self, x, y, button=1):
        self.xdo("mousemove", "--window", self.window, x, y, "click", button)
        time.sleep(.5)

    def key(self, *keys):
        self.xdo("key", "--clearmodifiers", *keys)
        time.sleep(.5)

    def type(self, text):
        self.xdo("type", "--clearmodifiers", "--delay", 90, text)
        time.sleep(.5)

    def grab(self):
        from PIL import ImageGrab
        image = ImageGrab.grab(xdisplay=self.env["DISPLAY"]).convert("RGB")
        image.save(self.output / "window.png")
        # Where the window is now: its window manager may have placed it
        # after the probe first asked.
        info = subprocess.check_output(["xwininfo", "-id", str(self.window)], env=self.env, text=True)
        origin = tuple(int(re.search(rf"Absolute upper-left {axis}:\s+(-?\d+)", info).group(1)) for axis in "XY")
        if origin != self.origin:
            print(f"the window moved from {self.origin} to {origin}", file=sys.stderr)
            self.origin = origin
        return lambda x, y: image.getpixel((self.origin[0] + x, self.origin[1] + y))

    def clipboard(self, text=None):
        """Another client's view of the clipboard: read it, or copy `text`."""
        if self.wayland:
            command = ["wl-copy"] if text is not None else ["wl-paste", "--no-newline"]
            env = self.wayland
        else:
            command = ["xclip", "-selection", "clipboard"] + (["-i"] if text is not None else ["-o"])
            env = self.env
        found = subprocess.run(command, env=env, input=text, capture_output=text is None, text=True, timeout=5)
        return found.stdout if text is None else None

    def lines(self, prefix):
        return [line[len(prefix):] for line in self.log.read_text().splitlines() if line.startswith(prefix)]

    def messages(self):
        out = []
        for line in self.lines("PAGE "):
            body, origin, main = re.fullmatch(r"(.*) origin=(\S*) main=(\w+)", line).groups()
            out.append((json.loads(body), origin, main == "true"))
        return out

    def wait(self, check, what, seconds=10):
        deadline = time.monotonic() + seconds
        while True:
            found = check()
            if found:
                return found
            if time.monotonic() > deadline:
                raise AssertionError(f"{what}; see application.log")
            time.sleep(.1)


def end(owned):
    """Stop what the probe started, last first; those in sessions of their
    own (D-Bus, the input method) with their children."""
    for process in reversed(owned):
        if os.getpgid(process.pid) == process.pid if process.poll() is None else False:
            try:
                os.killpg(process.pid, signal.SIGTERM)
            except (ProcessLookupError, PermissionError):
                pass
        stop(process)


MEDIA = """<!doctype html><html><body style="margin: 0; background: black">
<video id="video" src="media.webm" muted autoplay loop style="width: 100%"></video><script>
const post = (value) => window.zgx.postMessage(value);
const video = document.getElementById('video');
let played = false;
video.addEventListener('timeupdate', () => {
  if (!played && video.currentTime > 0.5) { played = true; post({ video: video.currentTime }); }
});
video.addEventListener('error', () => post({ video_error: video.error && video.error.code }));
addEventListener('click', async () => {
  const context = new AudioContext();
  const tone = context.createOscillator();
  tone.connect(context.destination);
  tone.start();
  await context.resume();
  setTimeout(() => post({ audio: context.state, time: context.currentTime }), 500);
});
</script></body></html>"""


def serve_media(output):
    """The media page and a WebM GStreamer encodes, over HTTP on a free
    localhost port (a file's media stays in WebKit's sandbox): its URL."""
    folder = output / "media"
    folder.mkdir(exist_ok=True)
    (folder / "index.html").write_text(MEDIA)
    subprocess.run(["gst-launch-1.0", "-q", "videotestsrc", "num-buffers=60", "pattern=ball", "!",
                    "video/x-raw,width=320,height=180,framerate=30/1", "!", "vp8enc", "deadline=1", "!",
                    "webmmux", "!", "filesink", "location=" + str(folder / "media.webm")], check=True, timeout=60)
    handler = functools.partial(http.server.SimpleHTTPRequestHandler, directory=str(folder))
    handler.log_message = lambda *_: None
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), handler)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    return f"http://127.0.0.1:{server.server_address[1]}/"


def media_checks(session):
    """The video plays half a second; a click on the page starts a tone."""
    checks = {}
    def message(key):
        return next((m[0] for m in session.messages() if key in m[0] or key + "_error" in m[0]), None)
    found = session.wait(lambda: message("video"), "the video never played (WebKit started no media)", 30)
    if "video" not in found:
        raise AssertionError(f"the video failed: {found}")
    checks["video played"] = f"{found['video']:.1f} s"
    session.click(340, 240)
    found = session.wait(lambda: message("audio"), "the AudioContext never answered", 15)
    if found.get("audio") != "running" or not found.get("time", 0) > 0:
        raise AssertionError(f"the AudioContext did not run: {found}")
    checks["audio ran"] = f"{found['audio']}, {found['time']:.2f} s"
    return checks


def page_shown(session, colours=None):
    """The page's frame as the window shows it (colours at their points)."""
    colours = colours or dict(COLOURS)
    def shown():
        pixel = session.grab()
        found = {name: pixel(*at) for name, (at, _) in colours.items()}
        return found if all(near(found[name], colour) for name, (_, colour) in colours.items()) else None
    try:
        return session.wait(shown, "the page did not show as expected")
    except AssertionError:
        pixel = session.grab()
        raise AssertionError("pixels: " + repr({name: (pixel(*at), colour) for name, (at, colour) in colours.items()}))


def inspector_checks(session):
    """Ctrl+I on a large page (one WebKit docks Web Inspector in): the
    inspector in a window of its own, never docked in the page even for a
    moment (docked, it would shrink the page's viewport, which the page
    reports as it resizes), the page as it was, and the inspector's console,
    typed into, reaching the page."""
    session.wait(lambda: session.lines("EVAL "), "the page never loaded", 30)
    # Low on the page, where a docked inspector would draw.
    colours = dict(COLOURS, lower=((340, 600), COLOURS["page"][1]))
    checks = {"pixels": page_shown(session, colours)}
    session.key("ctrl+i")
    if session.wayland:
        sway = dict(session.wayland, SWAYSOCK=str(next(pathlib.Path(session.wayland["XDG_RUNTIME_DIR"]).glob("sway-ipc*.sock"))))
        def windows():
            tree = json.loads(subprocess.run(["swaymsg", "-t", "get_tree", "-r"], env=sway, capture_output=True, text=True).stdout)
            found, pending = [], [tree]
            while pending:
                node = pending.pop()
                pending.extend(node.get("nodes", []) + node.get("floating_nodes", []))
                if (node.get("name") or "").startswith("Web Inspector"):
                    found.append(node["name"])
            return found
        checks["inspector window"] = session.wait(windows, "Web Inspector never opened in a window of its own", 30)[0]
        focus = ["swaymsg", '[title="^Web Inspector"] focus']
        raise_probe = ["swaymsg", '[title="^Web view probe$"] focus']
    else:
        def windows():
            return subprocess.run(["xdotool", "search", "--onlyvisible", "--name", "Web Inspector"],
                                  env=session.env, capture_output=True, text=True).stdout.split()
        inspector = session.wait(windows, "Web Inspector never opened in a window of its own", 30)[0]
        checks["inspector window"] = subprocess.run(["xdotool", "getwindowname", inspector], env=session.env,
                                                    capture_output=True, text=True).stdout.strip()
        focus = ["xdotool", "windowactivate", "--sync", inspector]
        raise_probe = ["xdotool", "windowactivate", "--sync", session.window]
        sway = session.env
    time.sleep(2)
    # The probe's window raised over it: the page as it was, nothing docked.
    subprocess.run(raise_probe, env=sway, capture_output=True)
    time.sleep(1)
    checks["page unchanged"] = page_shown(session, colours)
    # Its console (Escape shows it), typed into, evaluates in the page.
    subprocess.run(focus, env=sway, capture_output=True)
    time.sleep(2)
    session.xdo("key", "Escape")
    time.sleep(1)
    session.xdo("type", "--delay", 40, "window.zgx.postMessage({inspected: true})")
    session.xdo("key", "Return")
    session.wait(lambda: any(m[0].get("inspected") for m in session.messages()),
                 "the inspector's console never reached the page", 15)
    checks["console reached the page"] = True
    heights = [m[0]["resized"] for m in session.messages() if "resized" in m[0]]
    if any(height < 650 for height in heights):
        raise AssertionError(f"Web Inspector docked in the page: its viewport went {heights}")
    checks["page viewport"] = heights or "never resized"
    return checks


def page_checks(session, ime, transparent=False):
    session.wait(lambda: session.lines("EVAL "), "the page never loaded", 30)
    colours = dict(COLOURS)
    if transparent:
        colours["page"] = (colours["page"][0], (255, 255, 0))
    checks = {}
    evaluated = session.lines("EVAL ")[0]
    if json.loads(evaluated) != "probe page 3":
        raise AssertionError(f"evaluated {evaluated}")
    checks["script evaluated"] = evaluated
    # The page's frame reaches the window a frame after it loads.
    def shown():
        pixel = session.grab()
        found = {name: pixel(*at) for name, (at, _) in colours.items()}
        return found if all(near(found[name], colour) for name, (_, colour) in colours.items()) else None
    try:
        checks["pixels"] = session.wait(shown, "the page did not show as expected")
    except AssertionError:
        pixel = session.grab()
        raise AssertionError("pixels: " + repr({name: (pixel(*at), colour) for name, (at, colour) in colours.items()}))
    if transparent:
        return checks
    if ime:
        return ime(session, checks)
    if not session.wayland:
        checks["cursors"] = cursor_checks(session)
    checks["dragged out"] = drag_checks(session)
    session.click(160, 180)
    click = session.wait(lambda: [m for m in session.messages() if "click" in m[0]], "the click never reached the page")[0]
    if click[0]["click"] != [100, 40] or click[1] != "https://probe.zgx.test" or not click[2]:
        raise AssertionError(f"click heard as {click}")
    checks["click"] = click[0]
    session.click(120, 80)
    session.type("hello")
    session.wait(lambda: any(m[0].get("value") == "hello" for m in session.messages()), "typing never reached the field")
    checks["typed"] = "hello"
    for key in ("ctrl+j", "ctrl+k", "ctrl+l"):
        session.key(key)
    # Ctrl+K reaches the app after a round trip through the page's process,
    # Ctrl+L (reserved) at once: either may come first.
    session.wait(lambda: {"k", "l"} <= set(session.lines("ACTION ")), "the app's shortcuts never fired")
    actions = sorted(session.lines("ACTION "))
    heard = [m[0]["key"] for m in session.messages() if m[0].get("ctrl") and m[0].get("key") != "Control"]
    # Ctrl+J: the page's. Ctrl+K: left alone, so the app's. Ctrl+L: reserved.
    if actions != ["k", "l"] or heard != ["j", "k"]:
        raise AssertionError(f"shortcuts: app {actions}, page {heard}")
    checks["keys"] = {"app": actions, "page": heard}
    # Copied in the page, read by another client; copied there, pasted here.
    session.key("ctrl+a")
    session.key("ctrl+c")
    copied = session.wait(lambda: session.clipboard() == "hello", "the page's copy never reached the clipboard")
    checks["copied"] = copied and "hello"
    session.clipboard("from elsewhere")
    time.sleep(.3)
    session.key("ctrl+a")
    session.key("ctrl+v")
    session.wait(lambda: any(m[0].get("value") == "from elsewhere" for m in session.messages()), "the clipboard never pasted into the page")
    checks["pasted"] = "from elsewhere"
    # The select's popup, a zgx menu: Down to Beta, Return chooses it.
    session.click(500, 75)
    session.key("Down")
    session.key("Return")
    session.wait(lambda: any(m[0].get("chose") == "Beta" for m in session.messages()), "the select's popup never chose")
    checks["chose"] = "Beta"
    for _ in range(3):
        session.click(300, 300, 5)
    scrolled = session.wait(lambda: [m for m in session.messages() if "scrolled" in m[0]], "the wheel never scrolled the page")
    checks["scrolled"] = scrolled[0][0]["scrolled"]
    return checks


def cursor_checks(session):
    """The page's cursors as X shows them, each after one move and nothing
    after it (WebKit sets a cursor after its web process placed the pointer):
    the theme's hand over the target, the page's own image over its box (16
    × 16 red, its hot spot at 4, 4), the arrow over the plain page."""
    import ctypes

    class Image(ctypes.Structure):
        _fields_ = [("x", ctypes.c_short), ("y", ctypes.c_short), ("width", ctypes.c_ushort),
                    ("height", ctypes.c_ushort), ("xhot", ctypes.c_ushort), ("yhot", ctypes.c_ushort),
                    ("serial", ctypes.c_ulong), ("pixels", ctypes.POINTER(ctypes.c_ulong)),
                    ("atom", ctypes.c_ulong), ("name", ctypes.c_char_p)]
    x11 = ctypes.CDLL("libX11.so.6")
    x11.XOpenDisplay.argtypes = [ctypes.c_char_p]
    x11.XOpenDisplay.restype = ctypes.c_void_p
    x11.XFree.argtypes = [ctypes.c_void_p]
    x11.XCloseDisplay.argtypes = [ctypes.c_void_p]
    fixes = ctypes.CDLL("libXfixes.so.3")
    fixes.XFixesGetCursorImage.argtypes = [ctypes.c_void_p]
    fixes.XFixesGetCursorImage.restype = ctypes.POINTER(Image)
    display = x11.XOpenDisplay(session.env["DISPLAY"].encode())

    def shown():
        image = fixes.XFixesGetCursorImage(display)
        try:
            c = image.contents
            return {"name": c.name.decode() if c.name else None, "size": (c.width, c.height),
                    "hot": (c.xhot, c.yhot), "first": c.pixels[0] & 0xffffffff}
        finally:
            x11.XFree(image)
    wanted = {
        "pointer": ((160, 180), lambda c: c["name"] == "pointer"),
        "image": ((460, 170), lambda c: c["size"] == (16, 16) and c["hot"] == (4, 4) and c["first"] == 0xffff0000),
        "default": ((340, 340), lambda c: c["name"] == "default"),
    }
    found = {}
    try:
        for label, ((x, y), good) in wanted.items():
            session.xdo("mousemove", "--window", session.window, x, y)
            try:
                found[label] = session.wait(lambda: (lambda c: c if good(c) else None)(shown()), "", 3)
            except AssertionError:
                raise AssertionError(f"the page's {label} cursor never showed (X shows {shown()})")
    finally:
        x11.XCloseDisplay(display)
    return {label: c["name"] or f"{c['size'][0]}x{c['size'][1]} image" for label, c in found.items()}


def drag_checks(session):
    """Drags out of the page, with real input: its link onto a zgx drop target
    beside it, then onto another app's (a GTK window right of the probe's,
    clear of it: a drag leaves zgx by its window's bounds), and its image
    onto one asking for PNG first. On Wayland the GTK window is a Wayland
    client of the same Sway, floated where its config puts it (800, 0)."""
    def screen(x, y):
        return session.origin[0] + x, session.origin[1] + y

    def drag(start, end):
        sx, sy = screen(*start)
        session.xdo("mousemove", sx, sy)
        time.sleep(.3)
        session.xdo("mousedown", 1)
        for step in range(1, 25):
            x = sx + (end[0] - sx) * step / 24
            y = sy + (end[1] - sy) * step / 24
            session.xdo("mousemove", int(x), int(y))
            time.sleep(.05)
        time.sleep(.5)
        session.xdo("mouseup", 1)
        time.sleep(.5)

    found = {}
    link, image, target = (80, 292), (430, 250), (705, 130)
    drag(link, screen(*target))
    found["onto zgx"] = session.wait(lambda: session.lines("DROPPED "), "the page's link never dropped on zgx's target")[0]
    if found["onto zgx"] != "https://example.com/dragged":
        raise AssertionError(f"zgx's target got {found['onto zgx']!r}")
    # The page hears its drag end, and GTK never tried a drag of its own.
    session.wait(lambda: any(m[0].get("dragend") for m in session.messages()), "the page never heard its drag end")
    if "gtk_drag" in session.log.read_text():
        raise AssertionError("GTK warned of a drag: see application.log")
    script = str(pathlib.Path(__file__).resolve().parent / "xdnd_target.py")
    for name, start, extra in (("link", link, []), ("image", image, ["--image"])):
        x, y = screen(790, 100)
        log = session.output / f"target-{name}.log"
        env = dict(session.wayland, GDK_BACKEND="wayland") if session.wayland else session.env
        with log.open("w") as out:
            target_app = subprocess.Popen([sys.executable, script, f"Page {name} drops", str(x), str(y)] + extra,
                                          env=env, stdout=out, stderr=subprocess.STDOUT)
        try:
            session.wait(lambda: "READY" in log.read_text(), f"the {name} drop target never opened", 15)
            time.sleep(1)
            if session.wayland:
                # Floated at (800, 0) by Sway's config, its default size.
                centre = screen(1000, 100)
            else:
                # Clear of the probe's window, then its centre, where its
                # window manager put it.
                subprocess.run(["xdotool", "search", "--name", f"^Page {name} drops$", "windowmove", "--sync", str(x), str(y)],
                               env=session.env, capture_output=True)
                geometry = subprocess.run(["xdotool", "search", "--name", f"^Page {name} drops$", "getwindowgeometry"],
                                          env=session.env, capture_output=True, text=True).stdout
                left, top = map(int, re.search(r"Position: (\d+),(\d+)", geometry).groups())
                width, height = map(int, re.search(r"Geometry: (\d+)x(\d+)", geometry).groups())
                centre = (left + width // 2, top + height // 2)
            drag(start, centre)
            got = session.wait(lambda: [l for l in log.read_text().splitlines() if l.startswith("RECEIVED")],
                               f"another app never took the page's {name}", 10)[0]
        finally:
            target_app.terminate()
            target_app.wait(5)
        if name == "link" and "https://example.com/dragged" not in got:
            raise AssertionError(f"the app got {got!r}")
        if name == "image" and not got.startswith("RECEIVED image/png 82 bytes 89504e47"):
            raise AssertionError(f"the app got {got!r} for the image")
        found[f"{name} onto an app"] = got[len("RECEIVED "):]
    if "gtk_drag" in session.log.read_text():
        raise AssertionError("GTK warned of a drag: see application.log")
    return found


def compose(session, checks):
    """Pinyin "nihao", then Space: 你好 composed in the page's field."""
    session.wait(lambda: any(m[0].get("composition") == "start" for m in session.messages()), "no composition started", 15)
    session.key("space")
    session.wait(lambda: any(m[0].get("value") == "你好" for m in session.messages()), "你好 never reached the field", 15)
    ended = [m[0] for m in session.messages() if m[0].get("composition") == "end"]
    if not ended or ended[-1].get("data") != "你好":
        raise AssertionError(f"composition ended as {ended}")
    checks["composed"] = "你好"
    return checks


def start_orca(env, output, owned):
    """A private AT-SPI bus, accessibility on, and Orca on it (its speech in
    its debug log, which needs no speech server)."""
    env["GSETTINGS_BACKEND"] = "memory"
    with (output / "a11y-bus.log").open("w") as log:
        owned.append(subprocess.Popen(["/usr/libexec/at-spi-bus-launcher", "--launch-immediately", "--a11y=1", "--screen-reader=1"],
                                      env=env, stdout=log, stderr=log, start_new_session=True))
    def bus(*call):
        return subprocess.run(["gdbus", "call", "--session", "--dest", "org.a11y.Bus", "--object-path", "/org/a11y/bus", "--method", *call],
                              env=env, capture_output=True, text=True)
    deadline = time.monotonic() + 10
    while True:
        address = bus("org.a11y.Bus.GetAddress")
        if address.returncode == 0:
            break
        if time.monotonic() > deadline:
            raise RuntimeError("the accessibility bus never answered: " + address.stderr)
        time.sleep(.1)
    for name in ("IsEnabled", "ScreenReaderEnabled"):
        bus("org.freedesktop.DBus.Properties.Set", "org.a11y.Status", name, "<true>")
    env["AT_SPI_BUS_ADDRESS"] = re.search(r"'([^']+)'", address.stdout).group(1)
    with (output / "orca.log").open("w") as log:
        orca = subprocess.Popen(["orca", "--replace", "--debug-file=" + str(output / "orca-debug.log")], env=env, stdout=log, stderr=log)
    owned.append(orca)
    deadline = time.monotonic() + 20
    while "Screen reader on." not in spoken(output):
        if time.monotonic() > deadline or orca.poll() is not None:
            raise RuntimeError("Orca never started; see orca.log")
        time.sleep(.2)


def spoken(output):
    """What Orca said so far (its debug log's speech)."""
    path = output / "orca-debug.log"
    text = path.read_text(errors="replace") if path.exists() else ""
    return [m.group(1) for m in re.finditer(r"SPEECH OUTPUT: '([^']*)'", text)]


def said(session, output, text, after=0):
    """Whether Orca said `text` (after its first `after` utterances). Its
    debug log reaches the file in blocks: a Shift, which it echoes, now and
    then fills one."""
    found = any(text in line for line in spoken(output)[after:])
    if not found and time.monotonic() - said.nudged > 2:
        said.nudged = time.monotonic()
        session.xdo("key", "shift")
    return found
said.nudged = 0.


def orca_checks(session, env, output, checks):
    """The page's tree under its box, and what Orca reads of it."""
    tree = subprocess.run([sys.executable, "-c", TREE], env=env, capture_output=True, text=True, timeout=60)
    (output / "tree.txt").write_text(tree.stdout + tree.stderr)
    if "EMBEDDED" not in tree.stdout:
        raise AssertionError("the page's tree is not under its box; see tree.txt")
    checks["tree"] = tree.stdout.strip().splitlines()[0]
    checks["heading in the tree"] = "Probe heading"
    # Orca reads a caret moved by a click only once it has the click's
    # button from AT-SPI's registry; a registry that sends none fails every
    # check below as a missed heading. Clicks on the app's own background
    # first, until Orca has one.
    def buttons():
        path = output / "orca-debug.log"
        return "EVENT MANAGER: mouse:button:" in (path.read_text(errors="replace") if path.exists() else "")
    deadline = time.monotonic() + 20
    while not buttons():
        if time.monotonic() > deadline:
            raise AssertionError("AT-SPI's registry reported no mouse buttons to Orca; see orca-debug.log")
        session.click(700, 250)
        time.sleep(2)
    checks["registry reports buttons"] = "mouse:button"
    # The page's plain background first (no field: Orca stays in browse
    # mode), below the heading: Orca reads where its caret lands, or its
    # previous-heading key reads the heading, in one utterance or two.
    def heading():
        said(session, output, "Probe heading")
        return re.search(r"Probe heading\.? heading level 1", " ".join(spoken(output)))
    # Pressed before the page's first event reached Orca, the key goes to
    # the app's script, which has none: then the heading's own text (Orca
    # reads the line its caret lands on), then the background again.
    for x, y in ((440, 340), (140, 250), (440, 340)):
        session.click(x, y)
        time.sleep(2)
        if heading():
            break
        session.key("shift+h")
        try:
            session.wait(heading, "", 6)
            break
        except AssertionError:
            pass
    session.wait(heading, "Orca never read the heading", 2)
    checks["heading"] = "Probe heading heading level 1."
    # The app's own button, then Tab into the page, as into a form: the
    # page's first field takes focus, and Orca names it (once, with no retry).
    session.click(705, 55)
    session.wait(lambda: said(session, output, "Before page push button."), "Orca never read the app's button", 15)
    before = len(spoken(output))
    session.key("Tab")
    session.wait(lambda: said(session, output, "Probe field entry.", before), "Orca never named the field Tab reached", 15)
    checks["tabbed into the field"] = "Probe field entry."
    return checks


# Over the accessibility bus: the probe's box (an internal frame named Web
# page) holds the page's document, which holds its heading, and the
# document's ancestors reach the box and the window. One application.
TREE = """
import gi, time
gi.require_version("Atspi", "2.0")
from gi.repository import Atspi
Atspi.init()
Atspi.set_timeout(1500, 3000)
def children(node):
    node.clear_cache()
    return [node.get_child_at_index(i) for i in range(node.get_child_count())]
def find(node, role, name=None, depth=0):
    if node is None or depth > 14:
        return None
    if node.get_role_name() == role and (name is None or node.get_name() == name):
        return node
    for child in children(node):
        found = find(child, role, name, depth + 1)
        if found:
            return found
deadline = time.monotonic() + 30
frame = document = None
while time.monotonic() < deadline:
    apps = [a for a in children(Atspi.get_desktop(0)) if a and a.get_name() == "web_view_probe"]
    frame = next(filter(None, (find(a, "internal frame", "Web page") for a in apps)), None)
    document = frame and find(frame, "document web")
    if document and find(document, "heading", "Probe heading"):
        break
    time.sleep(.5)
chain, node = [], document
while node is not None and len(chain) < 10:
    node.clear_cache()
    node = node.get_parent()
    if node is not None:
        chain.append(node.get_role_name() + " " + repr(node.get_name()))
print("chain: " + " <- ".join(chain))
if document and "internal frame 'Web page'" in chain and "frame 'Web view probe'" in chain:
    print("EMBEDDED")
"""


def x11(args, output, private):
    env = dict(os.environ, XDG_RUNTIME_DIR=private, WINIT_X11_SCALE_FACTOR="1", LANG="C.UTF-8",
               XDG_CONFIG_HOME=private + "/config", XDG_CACHE_HOME=private + "/cache", XDG_DATA_HOME=private + "/data")
    for name in ("WAYLAND_DISPLAY", "IBUS_ADDRESS", "GTK_IM_MODULE", "XMODIFIERS"):
        env.pop(name, None)
    owned = []
    try:
        with (output / "xvfb.log").open("w") as log:
            xvfb = subprocess.Popen(["Xvfb", "-r", "-noreset", "-displayfd", "1", "-screen", "0", "1400x900x24", "-ac"],
                                    stdout=subprocess.PIPE, stderr=log, text=True)
        owned.append(xvfb)
        if not select.select([xvfb.stdout], [], [], 30)[0]:
            raise RuntimeError("Xvfb startup timed out")
        env["DISPLAY"] = ":" + xvfb.stdout.readline().strip()
        ime = None
        if args.ime or args.orca:
            with (output / "dbus.log").open("w") as log:
                bus = subprocess.Popen(["dbus-daemon", "--session", "--nofork", "--print-address=1"], stdout=subprocess.PIPE,
                                       stderr=log, env=env, text=True, start_new_session=True)
            owned.append(bus)
            if not select.select([bus.stdout], [], [], 10)[0]:
                raise RuntimeError("D-Bus startup timed out")
            env["DBUS_SESSION_BUS_ADDRESS"] = bus.stdout.readline().strip()
        if args.ime:
            memconf = next(p for p in ("/usr/libexec/ibus-memconf", "/usr/lib/ibus/ibus-memconf") if os.access(p, os.X_OK))
            env.update(XMODIFIERS="@im=ibus", IBUS_ENABLE_SYNC_MODE="0")
            subprocess.run(["gsettings", "set", "org.freedesktop.ibus.general", "use-system-keyboard-layout", "true"], env=env, check=True)
        with (output / "openbox.log").open("w") as log:
            owned.append(subprocess.Popen(["openbox"], env=env, stdout=log, stderr=log))
        wait_for_wm(env)
        if args.ime:
            with (output / "ibus.log").open("w") as log:
                ime = subprocess.Popen(["ibus-daemon", "--replace", "--cache=none", "--emoji-extension=disable",
                                        "--config=" + memconf, "--xim"], env=env, stdout=log, stderr=log, start_new_session=True)
            owned.append(ime)
            wait_for_ibus_xim(env)
        if args.orca:
            start_orca(env, output, owned)
        extra = ["--transparent"] if args.transparent else []
        # A page WebKit would dock Web Inspector in (`on_inspector_attach`).
        extra += ["--large-page"] if args.inspector else []
        extra += ["--media", serve_media(output)] if args.media else []
        with (output / "application.log").open("w") as log:
            app = subprocess.Popen([str(args.binary.resolve())] + extra, env=env, stdout=log, stderr=subprocess.STDOUT)
        owned.append(app)
        deadline = time.monotonic() + 20
        while True:
            found = subprocess.run(["xdotool", "search", "--onlyvisible", "--pid", str(app.pid)], env=env, capture_output=True, text=True)
            if found.returncode == 0 and found.stdout.strip():
                window = found.stdout.split()[0]
                break
            if time.monotonic() > deadline or app.poll() is not None:
                raise RuntimeError("the probe's window never opened; see application.log")
            time.sleep(.1)
        subprocess.run(["xdotool", "windowactivate", "--sync", window], env=env, check=True)
        info = subprocess.check_output(["xwininfo", "-id", window], env=env, text=True)
        origin = tuple(int(re.search(rf"Absolute upper-left {axis}:\s+(-?\d+)", info).group(1)) for axis in "XY")
        session = Session(output, env, window, origin)

        def ime_x11(session, checks):
            session.click(120, 80)
            use_ibus_engine(env, "libpinyin", ime)
            session.type("nihao")
            # IBus's candidates show under the page's caret, in its field
            # (window points 60 to 360 across, 60 to 100 down).
            def candidates():
                found = []
                listed = subprocess.run(["xdotool", "search", "--onlyvisible", "--class", "Ibus-ui-gtk3"], env=env, capture_output=True, text=True)
                for candidate in listed.stdout.split():
                    try:
                        info = subprocess.check_output(["xwininfo", "-id", candidate], env=env, text=True)
                    except subprocess.CalledProcessError:
                        continue
                    box = [int(re.search(pattern, info).group(1)) for pattern in (r"Absolute upper-left X:\s+(-?\d+)", r"Absolute upper-left Y:\s+(-?\d+)", r"Width:\s+(\d+)", r"Height:\s+(\d+)")]
                    x, y = box[0] - origin[0], box[1] - origin[1]
                    if box[2] > 100 and box[3] > 20 and 60 <= x <= 360 and 80 <= y <= 140:
                        found.append((x, y))
                return found
            checks["candidates at the caret"] = session.wait(candidates, "IBus's candidates are not under the page's caret")[0]
            return compose(session, checks)
        def read(session, checks):
            return orca_checks(session, env, output, checks)
        if args.media:
            return media_checks(session), "X11"
        if args.inspector:
            return inspector_checks(session), "X11"
        check = ime_x11 if args.ime else read if args.orca else None
        return page_checks(session, check, args.transparent), "X11"
    finally:
        end(owned)


def wayland(args, output, private):
    env = dict(os.environ, XDG_RUNTIME_DIR=private, LANG="C.UTF-8", XDG_CONFIG_HOME=private + "/config",
               XDG_CACHE_HOME=private + "/cache", XDG_DATA_HOME=private + "/data")
    for name in ("DISPLAY", "WAYLAND_DISPLAY", "SWAYSOCK", "IBUS_ADDRESS", "XMODIFIERS", "GTK_IM_MODULE", "QT_IM_MODULE"):
        env.pop(name, None)
    owned = []
    try:
        with (output / "xvfb.log").open("w") as log:
            xvfb = subprocess.Popen(["Xvfb", "-r", "-noreset", "-displayfd", "1", "-screen", "0", "1280x960x24", "-ac"],
                                    stdout=subprocess.PIPE, stderr=log, text=True)
        owned.append(xvfb)
        if not select.select([xvfb.stdout], [], [], 30)[0]:
            raise RuntimeError("Xvfb startup timed out")
        env["DISPLAY"] = ":" + xvfb.stdout.readline().strip()
        # A private session bus: Fcitx5 talks on it, and nothing else starts.
        bus_config = pathlib.Path(private, "dbus.conf")
        bus_config.write_text("<busconfig><type>session</type><listen>unix:tmpdir=" + private + "</listen><auth>EXTERNAL</auth>"
                              '<policy context="default"><allow send_destination="*"/><allow receive_sender="*"/><allow own="*"/></policy></busconfig>')
        with (output / "dbus.log").open("w") as log:
            bus = subprocess.Popen(["dbus-daemon", "--config-file=" + str(bus_config), "--nofork", "--print-address=1"],
                                   stdout=subprocess.PIPE, stderr=log, env=env, text=True, start_new_session=True)
        owned.append(bus)
        if not select.select([bus.stdout], [], [], 10)[0]:
            raise RuntimeError("D-Bus startup timed out")
        env["DBUS_SESSION_BUS_ADDRESS"] = bus.stdout.readline().strip()
        with (output / "openbox.log").open("w") as log:
            owned.append(subprocess.Popen(["openbox"], env=env, stdout=log, stderr=log))
        wait_for_wm(env)
        if args.ime:
            profile = pathlib.Path(private, "config/fcitx5/profile")
            profile.parent.mkdir(parents=True)
            profile.write_text("[Groups/0]\nName=Default\nDefault Layout=us\nDefaultIM=pinyin\n\n[Groups/0/Items/0]\nName=keyboard-us\n"
                               "Layout=\n\n[Groups/0/Items/1]\nName=pinyin\nLayout=\n\n[GroupOrder]\n0=Default\n")
        # The probe's window fills the output: its points are the output's.
        config = pathlib.Path(private, "sway.conf")
        sway_config(config, "xwayland disable\nseat seat0 fallback true\noutput * mode 1260x650\ndefault_border none\n"
                          "default_floating_border none\nfor_window [title=\"^Web view probe$\"] floating enable, move position 0 0\n"
                          "for_window [title=\"^Page .* drops$\"] floating enable, move position 800 0\n"
                          if not args.inspector else
                          # Room for the probe's large page and, clear of it,
                          # Web Inspector's own window.
                          "xwayland disable\nseat seat0 fallback true\noutput * mode 1260x900\ndefault_border none\n"
                          "default_floating_border none\nfor_window [title=\"^Web view probe$\"] floating enable, move position 0 0\n"
                          "for_window [title=\"^Web Inspector\"] floating enable, move position 1080 0\n")
        sway_env = dict(env, WLR_BACKENDS="x11", WLR_X11_OUTPUTS="1", WLR_RENDERER="pixman")
        with (output / "sway.log").open("w") as log:
            compositor = subprocess.Popen(["sway", "--unsupported-gpu", "--config", str(config)], env=sway_env, stdout=log, stderr=log)
        owned.append(compositor)
        window = wait_for_sway_host(env, output, compositor)
        deadline = time.monotonic() + 10
        while not [p for p in pathlib.Path(private).glob("wayland-*") if p.suffix != ".lock"]:
            if time.monotonic() > deadline:
                raise RuntimeError("Sway's socket never appeared")
            time.sleep(.05)
        socket = [p for p in pathlib.Path(private).glob("wayland-*") if p.suffix != ".lock"][0].name
        native = dict(env, WAYLAND_DISPLAY=socket, WINIT_UNIX_BACKEND="wayland")
        native.pop("DISPLAY")
        if args.ime:
            with (output / "fcitx.log").open("w") as log:
                ime = subprocess.Popen(["fcitx5", "-D", "--disable=xcb"], env=native, stdout=log, stderr=log, start_new_session=True)
            owned.append(ime)
            deadline = time.monotonic() + 10
            while subprocess.run(["fcitx5-remote", "--check"], env=env, capture_output=True).returncode:
                if time.monotonic() > deadline or ime.poll() is not None:
                    raise RuntimeError("Fcitx startup failed")
                time.sleep(.1)
        with (output / "application.log").open("w") as log:
            extra = (["--transparent"] if args.transparent else []) + (["--large-page"] if args.inspector else [])
            app = subprocess.Popen([str(args.binary.resolve())] + extra, env=native, stdout=log, stderr=subprocess.STDOUT)
        owned.append(app)
        subprocess.run(["xdotool", "windowfocus", "--sync", window], env=env, check=False)
        info = subprocess.check_output(["xwininfo", "-id", str(window)], env=env, text=True)
        origin = tuple(int(re.search(rf"Absolute upper-left {axis}:\s+(-?\d+)", info).group(1)) for axis in "XY")
        session = Session(output, env, window, origin, wayland=native)

        def ime_wayland(session, checks):
            session.click(120, 80)
            deadline = time.monotonic() + 10
            while subprocess.run(["fcitx5-remote", "-s", "pinyin"], env=env, capture_output=True).returncode:
                if time.monotonic() > deadline:
                    raise RuntimeError("Fcitx never loaded Pinyin")
                time.sleep(.1)
            subprocess.run(["fcitx5-remote", "-o"], env=env, check=True)
            time.sleep(.3)
            for character in "nihao":
                session.xdo("type", "--clearmodifiers", "--delay", 90, character)
                time.sleep(.3)
            return compose(session, checks)
        if args.inspector:
            return inspector_checks(session), "Wayland"
        return page_checks(session, ime_wayland if args.ime else None, args.transparent), "Wayland"
    finally:
        end(owned)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=pathlib.Path)
    parser.add_argument("--output", type=pathlib.Path, required=True)
    parser.add_argument("--wayland", action="store_true", help="the app on Wayland")
    parser.add_argument("--ime", action="store_true", help="Pinyin composed into the page's field")
    parser.add_argument("--transparent", action="store_true", help="a transparent page over its box's colour")
    parser.add_argument("--orca", action="store_true", help="the page's accessible tree, and Orca reading it (X11)")
    parser.add_argument("--media", action="store_true", help="a video plays and an AudioContext runs (X11)")
    parser.add_argument("--inspector", action="store_true", help="Web Inspector opens in a window of its own and takes input")
    parser.add_argument("--slow-replies", action="store_true",
                        help="WebKit's answers come 800 ms late (ZGX_WEBKITGTK_REPLY_DELAY_MS, debug builds only): on keys, after Ctrl is up; on the caret, after its frame")
    args = parser.parse_args()
    needed = ["Xvfb", "openbox", "xdotool", "xwininfo"]
    needed += ["sway", "dbus-daemon", "wl-copy", "wl-paste"] + (["fcitx5", "fcitx5-remote"] if args.ime else []) if args.wayland else []
    needed += ["xclip"] if not args.wayland and not args.ime and not args.orca and not args.media and not args.inspector else []
    needed += ["orca", "gdbus", "dbus-daemon"] if args.orca else []
    needed += ["gst-launch-1.0"] if args.media else []
    needed += ["ibus-daemon", "ibus", "dbus-daemon", "gsettings"] if args.ime and not args.wayland else []
    for name in needed:
        if shutil.which(name) is None:
            parser.error("Missing executable: " + name)
    args.output.mkdir(parents=True, exist_ok=True)
    (args.output / "result.json").unlink(missing_ok=True)
    if args.slow_replies:
        os.environ["ZGX_WEBKITGTK_REPLY_DELAY_MS"] = "800"
    with private_runtime("zgx-web-view-") as private:
        checks, backend = (wayland if args.wayland else x11)(args, args.output, private)
    result = {"result": "pass", "backend": backend + (" IME" if args.ime else "") + (" Orca" if args.orca else "") + (" media" if args.media else "") + (" transparent" if args.transparent else ""), "checks": checks}
    (args.output / "result.json").write_text(json.dumps(result, indent=2, ensure_ascii=False) + "\n")
    print("PASS: " + ", ".join(checks))


if __name__ == "__main__":
    main()
