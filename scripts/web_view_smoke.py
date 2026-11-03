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
console, typed into, reaches the page. `--unaccelerated`: the page painted
on the CPU (`WebViewOptions::accelerated` false) passes the same checks,
and two dots far apart that it turns green upload as two small parts.
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
# The page's two dots (F8 turns them green), far apart.
DOTS = ((345, 195), (605, 295))


def near(found, wanted, tolerance=24):
    return all(abs(a - b) <= tolerance for a, b in zip(found[:3], wanted))


class Session:
    def __init__(self, output, env, window, origin=(0, 0), wayland=None):
        self.output, self.env, self.window, self.origin = output, env, window, origin
        self.wayland = wayland
        # The probe's own lines (stdout), apart from its stderr: GLib's
        # warnings come in pieces, and one of ours landing between two
        # started no line of its own (`PAGE` mid-way through an Avahi
        # warning). The `WEB_` diagnostics are written to stderr in one go.
        self.log = output / "application.log"
        self.stderr = output / "stderr.log"

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
        log = self.stderr if prefix.startswith("WEB_") else self.log
        text = log.read_text(errors="replace") if log.exists() else ""
        return [line[len(prefix):] for line in text.splitlines() if line.startswith(prefix)]

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
                raise AssertionError(f"{what}; see application.log and stderr.log")
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


def save_folder(private):
    """Where the probe's download handler saves (`ZGX_PROBE_SAVE_DIR`)."""
    folder = pathlib.Path(private, "saved")
    folder.mkdir(exist_ok=True)
    return str(folder)


def downloads(private):
    """The session's downloads folder, by its XDG user directory: where
    WebKit's own handling would save a page's download."""
    folder = pathlib.Path(private, "downloads")
    folder.mkdir(exist_ok=True)
    pathlib.Path(private, "config").mkdir(exist_ok=True)
    pathlib.Path(private, "config", "user-dirs.dirs").write_text(f'XDG_DOWNLOAD_DIR="{folder}"\n')
    return folder


def damage_checks(session):
    """Painted on the CPU, the page uploads what changed: F8 turns its two
    dots green, both show, and no upload since was more than a tenth of
    the page (one box around both would be about one)."""
    page = PAGE[2] * PAGE[3]
    before = len(session.lines("WEB_DAMAGE "))
    session.key("F8")
    def green():
        pixel = session.grab()
        return all(near(pixel(x, y), (0, 255, 0)) for x, y in DOTS)
    session.wait(green, "the page's dots never turned green")
    uploads = [[tuple(int(n) for n in re.split("[,x]", part)) for part in line.split()]
               for line in session.lines("WEB_DAMAGE ")[before:]]
    if not uploads:
        raise AssertionError("no upload logged after F8")
    largest = max(w * h for parts in uploads for (_, _, w, h) in parts)
    if largest > page // 10:
        raise AssertionError(f"an upload took {largest} of the page's {page} pixels: {uploads}")
    return {"uploads": len(uploads), "largest part": largest}


def file_checks(session):
    """The page's file input opens the window's own file dialog (with no
    portal here, zenity's chooser): the file typed into it reaches the
    page, its name as the input's."""
    chosen = session.output.resolve() / "chosen.txt"
    chosen.write_text("chosen for the page\n")
    session.click(110, 352)
    def chooser():
        found = subprocess.run(["xdotool", "search", "--onlyvisible", "--name", "^Choose Files$"], env=session.env,
                               capture_output=True, text=True)
        return found.stdout.split()[0] if found.returncode == 0 and found.stdout.strip() else None
    window = session.wait(chooser, "the page's file input opened no file dialog", 20)
    subprocess.run(["xdotool", "windowactivate", "--sync", window], env=session.env)
    time.sleep(1)
    # The location entry, then the path: a Return may only complete it.
    session.xdo("key", "--clearmodifiers", "ctrl+l")
    time.sleep(.5)
    session.xdo("type", "--clearmodifiers", "--delay", "20", str(chosen))
    time.sleep(.5)
    def heard():
        return [m[0]["files"] for m in session.messages() if "files" in m[0]]
    for _ in range(3):
        session.xdo("key", "--clearmodifiers", "Return")
        try:
            session.wait(heard, "", 4)
            break
        except AssertionError:
            if not chooser():
                raise
    files = session.wait(heard, "the chosen file never reached the page's file input")[0]
    if files != ["chosen.txt"]:
        raise AssertionError(f"the page's file input took {files}")
    return files


def print_checks(session):
    """The app's print (Ctrl+P, reserved) and the page's `window.print()`
    (F7): GTK's print dialog, transient for the app's window (GTK's own
    choice, the page's offscreen window, leaves it anywhere), closed by
    Escape."""
    def dialog():
        found = subprocess.run(["xdotool", "search", "--onlyvisible", "--name", "^Print$"], env=session.env,
                               capture_output=True, text=True)
        return found.stdout.split()[0] if found.returncode == 0 and found.stdout.strip() else None
    done = []
    for how, key in (("window.print()", "F7"), ("print", "ctrl+p")):
        # The page's key, to the page: focus back in it after a dialog.
        session.click(*COLOURS["page"][0])
        session.key(key)
        if key == "F7":
            session.wait(lambda: any(m[0].get("printing") for m in session.messages()), "F7 never reached the page")
        window = session.wait(dialog, f"{how} showed no print dialog", 30)
        # The transient hint comes as the dialog shows.
        def transient():
            found = subprocess.run(["xprop", "-id", window, "WM_TRANSIENT_FOR"], env=session.env,
                                   capture_output=True, text=True).stdout
            match = re.search(r"window id # (0x[0-9a-f]+)", found)
            return match and int(match.group(1), 16) == int(session.window)
        session.wait(transient, f"{how}'s print dialog is not transient for the app's window", 10)
        subprocess.run(["xdotool", "windowactivate", "--sync", window], env=session.env)
        session.xdo("key", "Escape")
        session.wait(lambda: not dialog(), f"{how}'s print dialog never closed", 10)
        done.append(how)
    return done


def drop_checks(session):
    """Drags into the page, as a browser's: the page's card dragged onto
    its zone drops there with the data the page gave it; on X11, a file
    another app drags onto the zone drops as a `File`."""
    def dropped(after):
        return [m[0] for m in session.messages() if "dropped" in m[0]][after:]
    def heard(key):
        return any(key in m[0] for m in session.messages())
    def move_until(points, key, what):
        """Move through `points`, again and again, each move waiting for
        the page to say `key`: as fast as the page and zgx answer, which
        a load slows (fixed timings let the drag start after the release)."""
        for _ in range(8):
            for x, y in points:
                session.xdo("mousemove", "--window", session.window, x, y)
                try:
                    return session.wait(lambda: heard(key), "", .4)
                except AssertionError:
                    pass
        raise AssertionError(what)
    session.xdo("mousemove", "--window", session.window, 515, 310)
    session.xdo("mousedown", "1")
    # WebKit's drag of the card begins (its `dragstart`) ...
    move_until(((520, 316), (524, 320), (520, 324)), "dragging", "the card's drag never began")
    # ... becomes zgx's at the next move, and goes over the zone.
    move_until(((510, 340), (495, 355), (490, 365), (492, 367)), "entered", "the card's drag never reached the zone")
    session.xdo("mouseup", "1")
    first = session.wait(lambda: dropped(0), "the card never dropped on the zone")[0]
    if first["dropped"] != "card-1":
        raise AssertionError(f"the zone took {first}")
    checks = {"card": first["dropped"]}
    if session.wayland:
        return checks
    # A file from another app (GTK's drag source, `native_file_source.py`).
    path = session.output.resolve() / "dropped.txt"
    path.write_text("dropped into the page\n")
    source = subprocess.Popen([sys.executable, str(pathlib.Path(__file__).with_name("native_file_source.py")), str(path)],
                              env=session.env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    try:
        def source_window():
            found = subprocess.run(["xdotool", "search", "--onlyvisible", "--name", "^zgx file source$"], env=session.env,
                                   capture_output=True, text=True)
            return found.stdout.split()[0] if found.returncode == 0 and found.stdout.strip() else None
        window = session.wait(source_window, "the file source never opened", 10)
        # Beside the probe, clear of it.
        session.xdo("windowmove", window, 900, 100)
        time.sleep(.5)
        session.xdo("mousemove", "--window", window, 80, 60)
        session.xdo("mousedown", "1")
        for x, y in ((110, 60), (160, 90), (60, 160)):
            session.xdo("mousemove", "--window", window, x, y)
            time.sleep(.15)
        session.xdo("mousemove", "--window", session.window, 490, 365)
        time.sleep(.6)
        session.xdo("mouseup", "1")
        second = session.wait(lambda: dropped(1), "the file never dropped on the zone", 15)[0]
        if second["dropped_files"] != [["dropped.txt", "text/plain", path.stat().st_size]]:
            raise AssertionError(f"the zone took {second}")
        checks["file"] = second["dropped_files"][0]
    finally:
        stop(source)
    return checks


def page_checks(session, ime, transparent=False, unaccelerated=False):
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
    checks["dropped in"] = drop_checks(session)
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
    if session.wayland:
        checks["rich clipboard"] = rich_clipboard_checks(session)
    # The select's popup, a zgx menu: Down to Beta, Return chooses it.
    session.click(500, 75)
    session.key("Down")
    session.key("Return")
    session.wait(lambda: any(m[0].get("chose") == "Beta" for m in session.messages()), "the select's popup never chose")
    checks["chose"] = "Beta"
    checks["context menu"] = context_menu_checks(session)
    # A link that downloads: refused, as macOS's and Windows's web views do
    # with no handler of the app's, so nothing lands in the downloads folder.
    session.click(530, 240)
    session.wait(lambda: any(m[0].get("save") for m in session.messages()), "the download link's click never reached the page")
    time.sleep(2)
    saved = sorted(p.name for p in downloads(session.env["XDG_RUNTIME_DIR"]).iterdir())
    if saved:
        raise AssertionError(f"the page's download was saved: {saved}")
    checks["download refused"] = True
    # With the app's handler (Ctrl+D), the same link saves where the app
    # says, and only there.
    session.key("ctrl+d")
    session.wait(lambda: session.lines("DOWNLOADS on"), "the app's download handler never came on")
    session.click(530, 240)
    session.wait(lambda: "Finished" in session.lines("DOWNLOAD_STATUS "), "the app's download never finished")
    saved = pathlib.Path(session.env["ZGX_PROBE_SAVE_DIR"], "zgx-saved.txt")
    asked = session.lines("DOWNLOAD ")
    if not saved.exists() or saved.read_text() != "saved" or not asked or not asked[0].startswith("zgx-saved.txt text/plain"):
        raise AssertionError(f"the app's download: asked {asked}, saved {saved.exists() and saved.read_text()!r}")
    if any(downloads(session.env["XDG_RUNTIME_DIR"]).iterdir()):
        raise AssertionError("a download landed in the downloads folder")
    checks["download saved by the app"] = asked[0]
    if unaccelerated:
        checks["uploads what changed"] = damage_checks(session)
    # Full screen: F9 asks the page's green target for it, which fills
    # the box and reaches the app's state; Ctrl+E (the app's
    # `exit_full_screen`) ends it, the page as it was.
    session.key("F9")
    session.wait(lambda: session.lines("FULL_SCREEN ")[-1:] == ["true"], "the page's full screen never reached its state")
    session.wait(lambda: near(session.grab()(*COLOURS["page"][0]), (0, 255, 0)), "the full-screen target never filled the box")
    session.key("ctrl+e")
    session.wait(lambda: session.lines("FULL_SCREEN ")[-1:] == ["false"], "the page never left full screen")
    session.wait(lambda: near(session.grab()(*COLOURS["page"][0]), COLOURS["page"][1]), "the page never came back from full screen")
    checks["full screen"] = True
    if not session.wayland:
        checks["file input"] = file_checks(session)
    # After the damage check: a dialog closing activates the window again,
    # which repaints the whole page once.
    if not session.wayland:
        checks["printed"] = print_checks(session)
    # Find (reserved keys, the app's): the heading's word, twice (the next
    # match), and a word the page hasn't.
    for key in ("ctrl+f", "ctrl+f", "ctrl+g"):
        session.key(key)
    finds = session.wait(lambda: len(session.lines("FIND ")) >= 3 and session.lines("FIND "), "the page's find never answered")
    if finds != ["Probe found=true matches=Some(1)"] * 2 + ["nowhere-in-the-page found=false matches=Some(0)"]:
        raise AssertionError(f"found {finds}")
    checks["found"] = finds
    for _ in range(3):
        session.click(300, 300, 5)
    scrolled = session.wait(lambda: [m for m in session.messages() if "scrolled" in m[0]], "the wheel never scrolled the page")
    checks["scrolled"] = scrolled[0][0]["scrolled"]
    return checks


def rich_clipboard_checks(session):
    """Wayland, where GTK's clipboard reaches nothing: the page's copy of a
    selection reaches other apps as HTML too, and an image the page copies
    as a PNG."""
    # The heading selected (a triple click), copied: its HTML too.
    session.click(90, 250)
    session.xdo("click", "--repeat", "3", "--delay", "80", "1")
    session.key("ctrl+c")
    def html():
        found = subprocess.run(["wl-paste", "--type", "text/html"], env=session.wayland, capture_output=True, text=True, timeout=5)
        return found.returncode == 0 and "Probe heading" in found.stdout
    session.wait(html, "the page's copy reached other apps without its HTML", 10)
    # An image the page copies (F6: a canvas's PNG, through the clipboard
    # API): other apps paste it.
    session.key("F6")
    copied = session.wait(lambda: [m[0]["copied"] for m in session.messages() if "copied" in m[0]], "the page's image copy never answered")[-1]
    if copied != "image":
        raise AssertionError(f"the page's image copy: {copied}")
    def png():
        found = subprocess.run(["wl-paste", "--type", "image/png"], env=session.wayland, capture_output=True, timeout=5)
        return found.returncode == 0 and found.stdout.startswith(b"\x89PNG")
    session.wait(png, "the page's copied image never reached other apps", 10)
    return {"html": True, "image": True}


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


def context_menu_checks(session):
    """A right click on the page's link: WebKit's menu for it, as a zgx
    menu, whose Copy Link Location copies the link's address."""
    before = len(session.lines("WEB_MENU "))
    session.click(70, 290, 3)
    menu = session.wait(lambda: session.lines("WEB_MENU ")[before:], "a right click showed no context menu")[0].split("|")
    if "Copy Link Location" not in menu:
        raise AssertionError(f"the link's context menu: {menu}")
    time.sleep(.5)
    # The menu opens on its first item.
    for _ in range(menu.index("Copy Link Location")):
        session.key("Down")
    session.key("Return")
    session.wait(lambda: session.clipboard() == "https://example.com/dragged", "Copy Link Location copied nothing")
    return menu


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
    if "gtk_drag" in session.stderr.read_text(errors="replace"):
        raise AssertionError("GTK warned of a drag: see stderr.log")
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
    if "gtk_drag" in session.stderr.read_text(errors="replace"):
        raise AssertionError("GTK warned of a drag: see stderr.log")
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


class Buttons:
    """The mouse button events AT-SPI's registry sends its listeners (Orca
    among them), heard by a listener of ours on the same bus: as they come,
    where Orca's debug log reaches its file in blocks."""
    def __init__(self, env, output):
        self.events = []
        self.log = (output / "buttons.log").open("w")
        self.process = subprocess.Popen([sys.executable, "-u", "-c", BUTTONS], env=env, stdout=subprocess.PIPE,
                                        stderr=self.log, text=True)
        def read():
            for line in self.process.stdout:
                self.log.write(line)
                self.log.flush()
                self.events.append(line.strip())
        threading.Thread(target=read, daemon=True).start()
        deadline = time.monotonic() + 10
        while "READY" not in self.events:
            if time.monotonic() > deadline or self.process.poll() is not None:
                raise AssertionError("Orca not ready: no AT-SPI listener could join the registry; see buttons.log")
            time.sleep(.05)

    def count(self, kind):
        return sum(event.endswith(kind) for event in self.events)

    def click(self, session, x, y, timeout=3):
        """Click at `x`, `y`, the button held until the registry reports
        its press, and its release seen too: the registry reads the
        pointer's buttons by polling it, so xdotool's click (a press and a
        release at once) can come and go between two polls, and Orca then
        never ties the caret it moved to a click, and says nothing. False
        if the registry never reported the press."""
        presses, releases = self.count("p"), self.count("r")
        session.xdo("mousemove", "--window", session.window, x, y, "mousedown", 1)
        deadline = time.monotonic() + timeout
        while self.count("p") == presses and time.monotonic() < deadline:
            time.sleep(.02)
        pressed = self.count("p") > presses
        session.xdo("mouseup", 1)
        deadline = time.monotonic() + timeout
        while pressed and self.count("r") == releases and time.monotonic() < deadline:
            time.sleep(.02)
        time.sleep(.5)
        return pressed

    def stop(self):
        self.process.kill()
        self.process.wait()
        self.log.close()


def orca_click(session, buttons, x, y, tries=3):
    """A click Orca hears as one (`Buttons.click`), tried again while the
    registry misses it: past `tries`, Orca isn't ready, and no check after
    it would say why."""
    for _ in range(tries):
        if buttons.click(session, x, y):
            return
        buttons.log.write(f"MISSED a click at ({x}, {y})\n")
    raise AssertionError(f"Orca not ready: AT-SPI's registry reported none of {tries} clicks at ({x}, {y}) "
                         "to its listeners; see buttons.log and orca-debug.log")


def orca_checks(session, env, output, checks):
    """The page's tree under its box, and what Orca reads of it."""
    tree = subprocess.run([sys.executable, "-c", TREE], env=env, capture_output=True, text=True, timeout=60)
    (output / "tree.txt").write_text(tree.stdout + tree.stderr)
    if "EMBEDDED" not in tree.stdout:
        raise AssertionError("the page's tree is not under its box; see tree.txt")
    checks["tree"] = tree.stdout.strip().splitlines()[0]
    checks["heading in the tree"] = "Probe heading"
    buttons = Buttons(env, output)
    try:
        return read_page(session, output, buttons, checks)
    finally:
        buttons.stop()


def read_page(session, output, buttons, checks):
    """What Orca reads of the page: each click one the registry reported
    (`orca_click`), so a missed heading is Orca's, not a lost click's."""
    # Orca reads a caret moved by a click only once it has the click's
    # button from AT-SPI's registry. A click on the app's own background
    # first: Orca's first button event.
    orca_click(session, buttons, 700, 250)
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
        orca_click(session, buttons, x, y)
        time.sleep(1.5)
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
    orca_click(session, buttons, 705, 55)
    session.wait(lambda: said(session, output, "Before page push button."), "Orca never read the app's button", 15)
    before = len(spoken(output))
    session.key("Tab")
    session.wait(lambda: said(session, output, "Probe field entry.", before), "Orca never named the field Tab reached", 15)
    checks["tabbed into the field"] = "Probe field entry."
    return checks


# The registry's mouse button events (`mouse:button:1p`, `…:1r`), a line
# each, once listening.
BUTTONS = """
import gi
gi.require_version("Atspi", "2.0")
from gi.repository import Atspi
Atspi.init()
listener = Atspi.EventListener.new(lambda event: print(event.type, flush=True))
listener.register("mouse:button")
print("READY", flush=True)
Atspi.event_main()
"""


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
    # No session bus but the private one IME and Orca runs start: a file
    # dialog is zenity's, which xdotool answers (no portal's).
    for name in ("WAYLAND_DISPLAY", "IBUS_ADDRESS", "GTK_IM_MODULE", "XMODIFIERS", "DBUS_SESSION_BUS_ADDRESS"):
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
        downloads(private)
        env["ZGX_WEB_MENU"] = "1"
        env["ZGX_PROBE_SAVE_DIR"] = save_folder(private)
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
        if args.unaccelerated:
            extra.append("--unaccelerated")
            env["ZGX_WEB_DAMAGE"] = "1"
        with (output / "application.log").open("w") as log, (output / "stderr.log").open("w") as errors:
            app = subprocess.Popen([str(args.binary.resolve())] + extra, env=env, stdout=log, stderr=errors)
        owned.append(app)
        deadline = time.monotonic() + 20
        while True:
            found = subprocess.run(["xdotool", "search", "--onlyvisible", "--pid", str(app.pid)], env=env, capture_output=True, text=True)
            if found.returncode == 0 and found.stdout.strip():
                window = found.stdout.split()[0]
                break
            if time.monotonic() > deadline or app.poll() is not None:
                raise RuntimeError("the probe's window never opened; see application.log and stderr.log")
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
        return page_checks(session, check, args.transparent, args.unaccelerated), "X11"
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
        downloads(private)
        env["ZGX_WEB_MENU"] = "1"
        env["ZGX_PROBE_SAVE_DIR"] = save_folder(private)
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
        with (output / "application.log").open("w") as log, (output / "stderr.log").open("w") as errors:
            extra = (["--transparent"] if args.transparent else []) + (["--large-page"] if args.inspector else [])
            if args.unaccelerated:
                extra.append("--unaccelerated")
                native["ZGX_WEB_DAMAGE"] = "1"
            app = subprocess.Popen([str(args.binary.resolve())] + extra, env=native, stdout=log, stderr=errors)
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
        return page_checks(session, ime_wayland if args.ime else None, args.transparent, args.unaccelerated), "Wayland"
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
    parser.add_argument("--unaccelerated", action="store_true", help="the page painted on the CPU, uploading what changed")
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
    result = {"result": "pass", "backend": backend + (" IME" if args.ime else "") + (" Orca" if args.orca else "") + (" media" if args.media else "") + (" transparent" if args.transparent else "") + (" unaccelerated" if args.unaccelerated else ""), "checks": checks}
    (args.output / "result.json").write_text(json.dumps(result, indent=2, ensure_ascii=False) + "\n")
    print("PASS: " + ", ".join(checks))


if __name__ == "__main__":
    main()
