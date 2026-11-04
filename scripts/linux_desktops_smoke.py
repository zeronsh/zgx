#!/usr/bin/env python3
"""A window and a popup on each Linux desktop this machine has, headless and
drawn in software, so none needs a GPU or a screen. A desktop not installed is
skipped and said to be: `docs/validation.md` lists which were run where.

On each, popup_probe's menu (`POPUP_PROBE_OPEN`, opened by the app: not every
compositor can be sent a click) shows, drawn, past the window's edge or within
it as the desktop places it.

`WindowCapabilities::blur_behind` says what each does: only KWin blurs (true
there, false on the rest). And on those that maximize a window it is asked to
(all but sway), `WindowContext::maximized` follows the system's word.

On KWin and sway, glass_counter's glass over a wallpaper of hard stripes: KWin,
which has a blur protocol, blurs them; sway, which has none, shows them sharp,
the difference `Glass` documents. Blur needs KWin's GL compositing, which
llvmpipe gives it. Nothing here says anything about GPU speed.

The desktops: KWin (`kwin_wayland --virtual`), GNOME Shell (`gnome-shell
--headless`), sway, labwc and Weston on Wayland; Openbox alone and with picom, xfwm4
with its compositor, kwin_x11 and Mutter on X11 under Xvfb."""
import argparse
import json
import os
import pathlib
import select
import shutil
import subprocess
import sys
import tempfile
import time

from PIL import Image, ImageChops, ImageDraw

sys.path.insert(0, str(pathlib.Path(__file__).parent))
from platform_smoke import release_mounts, stop  # noqa: E402

SCREEN = (1400, 900)
# popup_probe's window and menu.
CONTENT = (16, 20, 28)
MENU = (35, 46, 64)
# What has a blur protocol, and what doesn't maximize a window that asks.
BLURS = {"kwin"}
NO_MAXIMIZE = {"sway"}
# Each pixel's difference from its neighbour, as a share of the wallpaper's: a
# blur takes most of it away.
BLURRED_BELOW, SHARP_ABOVE = .25, .5


def box(image, colour):
    """The box of `image`'s pixels of exactly `colour`."""
    solid = Image.new("RGB", image.size, colour)
    return ImageChops.difference(image, solid).convert("L").point(lambda v: 255 if v == 0 else 0).getbbox()


def stripes(path):
    """A wallpaper that shows blur: fine stripes on hard-edged colours."""
    image = Image.new("RGB", SCREEN)
    draw = ImageDraw.Draw(image)
    colours = [(230, 70, 60), (250, 190, 50), (70, 180, 110), (60, 120, 230), (240, 240, 240)]
    for y in range(0, SCREEN[1], 90):
        for x in range(0, SCREEN[0], 90):
            draw.rectangle([x, y, x + 90, y + 90], fill=colours[(x // 90 + 2 * (y // 90)) % len(colours)])
    for x in range(0, SCREEN[0], 6):
        draw.line([(x, 0), (x, SCREEN[1])], fill=(20, 20, 20), width=1)
    image.save(path)


def roughness(image, area):
    """How much each pixel of `area` differs from the next, on average."""
    pixels = image.crop(area).convert("L")
    width, height = pixels.size
    data = pixels.tobytes()
    total = sum(abs(data[y * width + x] - data[y * width + x + 1])
                for y in range(height) for x in range(width - 1))
    return total / max(1, (width - 1) * height)


def descendants(pid):
    """Every process under `pid`, deepest first: what a desktop started."""
    children = {}
    for entry in pathlib.Path("/proc").iterdir():
        if entry.name.isdigit():
            try:
                stat = (entry / "stat").read_text()
            except OSError:
                continue
            children.setdefault(int(stat.rsplit(")", 1)[1].split()[1]), []).append(int(entry.name))
    found = []
    def walk(parent):
        for child in children.get(parent, []):
            walk(child)
            found.append(child)
    walk(pid)
    return found


class Desktop:
    """One private desktop: its processes, environment and capture."""

    def __init__(self, name, output):
        self.name, self.output = name, output
        self.processes = []
        self.runtime = pathlib.Path(tempfile.mkdtemp(prefix="zgx-desktop-", dir=output))
        self.runtime.chmod(0o700)
        home = self.runtime / "home"
        (home / ".config").mkdir(parents=True)
        self.env = {
            "PATH": os.environ["PATH"], "HOME": str(home), "XDG_RUNTIME_DIR": str(self.runtime),
            "XDG_CONFIG_HOME": str(home / ".config"), "LANG": "en_US.UTF-8", "ZGX_RENDER_WHEN_COVERED": "1",
        }
        # With no DRM device Mesa's device-select layer waits for ever on an
        # answer about one from a compositor that has none to give (KWin).
        if not os.path.exists("/dev/dri"):
            self.env["VK_LOADER_LAYERS_DISABLE"] = "*MESA_device_select*"

    def start(self, label, command, **options):
        log = (self.output / f"{self.name}-{label}.log").open("w")
        env = dict(self.env, **options.pop("env", {}))
        process = subprocess.Popen(list(map(str, command)), env=env, stdout=log, stderr=subprocess.STDOUT,
                                   stdin=subprocess.DEVNULL, **options)
        self.processes.append(process)
        return process

    def bus(self):
        address = f"unix:path={self.runtime}/bus"
        self.env["DBUS_SESSION_BUS_ADDRESS"] = address
        self.start("dbus", ["dbus-daemon", "--session", f"--address={address}", "--nofork", "--nopidfile"])
        self.wait("the session bus", lambda: (self.runtime / "bus").exists())

    def xvfb(self):
        xvfb = subprocess.Popen(["Xvfb", "-noreset", "-displayfd", "1", "-screen", "0",
                                 "%dx%dx24" % SCREEN, "-ac"], env=self.env, stdout=subprocess.PIPE,
                                stderr=subprocess.DEVNULL, text=True)
        self.processes.append(xvfb)
        if not select.select([xvfb.stdout], [], [], 30)[0]:
            raise RuntimeError("Xvfb startup timed out")
        self.env["DISPLAY"] = ":" + xvfb.stdout.readline().strip()

    def wait(self, what, test, seconds=20):
        deadline = time.monotonic() + seconds
        while True:
            value = test()
            if value:
                return value
            if time.monotonic() > deadline:
                raise TimeoutError(f"{self.name}: timed out waiting for {what}")
            time.sleep(.1)

    def socket(self, name=None):
        """Wait for the compositor's socket; `WAYLAND_DISPLAY` from now on."""
        def found():
            if name:
                return (self.runtime / name).is_socket() and name
            names = [p.name for p in self.runtime.glob("wayland-*") if p.suffix != ".lock"]
            return names[0] if names else None
        self.env["WAYLAND_DISPLAY"] = self.wait("its socket", found, 30)

    def run(self, *command):
        return subprocess.run(list(map(str, command)), env=self.env, capture_output=True, text=True)

    def stop(self):
        for process in reversed(self.processes):
            for pid in descendants(process.pid):
                try:
                    os.kill(pid, 15)
                except OSError:
                    pass
            stop(process)
        release_mounts(self.runtime)
        shutil.rmtree(self.runtime, ignore_errors=True)


def x11(window_manager, *compositor):
    def start(desktop):
        desktop.xvfb()
        desktop.bus()
        desktop.start("wm", window_manager)
        time.sleep(1.5)
        if compositor:
            desktop.start("compositor", list(compositor))
            time.sleep(1)
    return start


def x11_shot(desktop, path):
    desktop.run("import", "-window", "root", path)


def sway(desktop):
    config = desktop.runtime / "sway.conf"
    config.write_text('default_border none\nfor_window [app_id=".*"] floating enable\n'
                      "output * mode %dx%d\nxwayland disable\n" % SCREEN)
    desktop.env.update(WLR_BACKENDS="headless", WLR_RENDERER="pixman", WLR_LIBINPUT_NO_DEVICES="1")
    desktop.bus()
    desktop.start("wm", ["sway", "--unsupported-gpu", "-c", config])
    desktop.socket()


def grim_shot(desktop, path):
    desktop.run("grim", path)


def labwc(desktop):
    desktop.env.update(WLR_BACKENDS="headless", WLR_RENDERER="pixman", WLR_LIBINPUT_NO_DEVICES="1",
                       WLR_HEADLESS_OUTPUTS="1")
    desktop.bus()
    desktop.start("wm", ["labwc"])
    desktop.socket()
    desktop.run("wlr-randr", "--output", "HEADLESS-1", "--custom-mode", "%dx%d" % SCREEN)


def weston(desktop):
    desktop.bus()
    desktop.start("wm", ["weston", "--backend=headless", "--renderer=gl", "--width=%d" % SCREEN[0],
                         "--height=%d" % SCREEN[1], "--socket=weston", "--debug"])
    desktop.socket("weston")


def weston_shot(desktop, path):
    subprocess.run(["weston-screenshooter"], env=desktop.env, cwd=desktop.runtime, capture_output=True)
    shot = next(desktop.runtime.glob("wayland-screenshot-*.png"))
    shot.replace(path)


def kwin(desktop):
    desktop.env.update(KWIN_SCREENSHOT_NO_PERMISSION_CHECKS="1", KWIN_WAYLAND_NO_PERMISSION_CHECKS="1",
                       XDG_CURRENT_DESKTOP="KDE")
    (pathlib.Path(desktop.env["XDG_CONFIG_HOME"]) / "kwinrc").write_text(
        "[Plugins]\nblurEnabled=true\n[Effect-blur]\nBlurStrength=8\n")
    desktop.bus()
    desktop.start("wm", ["kwin_wayland", "--virtual", "--no-lockscreen", "--width", SCREEN[0],
                         "--height", SCREEN[1], "--socket", "kwin"])
    desktop.socket("kwin")
    time.sleep(2)
    desktop.run("gdbus", "call", "--session", "-d", "org.kde.KWin", "-o", "/Effects", "-m",
                "org.kde.kwin.Effects.loadEffect", "blur")


KWIN_SHOT = """
import os, sys, dbus
from dbus.types import UnixFd
from PIL import Image
shots = dbus.Interface(dbus.SessionBus().get_object("org.kde.KWin", "/org/kde/KWin/ScreenShot2"),
                       "org.kde.KWin.ScreenShot2")
read, write = os.pipe()
meta = shots.CaptureWorkspace({"native-resolution": True, "include-cursor": False}, UnixFd(write), timeout=30000)
os.close(write)
data = b""
while chunk := os.read(read, 1 << 20):
    data += chunk
size = int(meta["width"]), int(meta["height"])
Image.frombuffer("RGBA", size, data, "raw", "BGRA", int(meta["stride"]), 1).convert("RGB").save(sys.argv[1])
"""


def kwin_shot(desktop, path):
    subprocess.run([sys.executable, "-c", KWIN_SHOT, str(path)], env=desktop.env, capture_output=True)


def gnome(desktop):
    desktop.env.update(XDG_CURRENT_DESKTOP="GNOME", XDG_SESSION_TYPE="wayland")
    desktop.bus()
    desktop.start("wm", ["gnome-shell", "--headless", "--virtual-monitor", "%dx%d" % SCREEN, "--wayland",
                         "--no-x11", "--wayland-display", "gnome", "--unsafe-mode"])
    desktop.socket("gnome")
    time.sleep(4)
    # It starts in its overview, over any window.
    desktop.run("gdbus", "call", "--session", "-d", "org.gnome.Shell", "-o", "/org/gnome/Shell", "-m",
                "org.gnome.Shell.Eval", "Main.overview.hide()")
    time.sleep(1)


def gnome_shot(desktop, path):
    desktop.run("gdbus", "call", "--session", "-d", "org.gnome.Shell.Screenshot", "-o",
                "/org/gnome/Shell/Screenshot", "-m", "org.gnome.Shell.Screenshot.Screenshot", "false", "false", path)


# name, what it needs installed, how it starts, how it is captured.
DESKTOPS = [
    ("kwin", ["kwin_wayland", "gdbus"], kwin, kwin_shot),
    ("gnome", ["gnome-shell", "gdbus"], gnome, gnome_shot),
    ("sway", ["sway", "grim"], sway, grim_shot),
    ("labwc", ["labwc", "grim", "wlr-randr"], labwc, grim_shot),
    ("weston", ["weston", "weston-screenshooter"], weston, weston_shot),
    ("openbox", ["openbox", "Xvfb", "import"], x11(["openbox"]), x11_shot),
    ("openbox-picom", ["openbox", "picom", "Xvfb", "import"],
     x11(["openbox"], "picom", "--backend", "xrender", "--shadow"), x11_shot),
    ("xfwm4", ["xfwm4", "Xvfb", "import"], x11(["xfwm4", "--compositor=on"]), x11_shot),
    ("kwin-x11", ["kwin_x11", "Xvfb", "import"], x11(["kwin_x11", "--no-kactivities"]), x11_shot),
    ("mutter-x11", ["mutter", "Xvfb", "import"], x11(["mutter", "--x11", "--sm-disable"]), x11_shot),
]


def popup(desktop, probe, capture, output, checks):
    """popup_probe's menu, opened by the app: is it drawn?"""
    name = desktop.name
    log = output / f"{name}-probe.log"
    app = subprocess.Popen([str(probe)], env=dict(desktop.env, POPUP_PROBE_OPEN="1"), stdout=log.open("w"),
                           stderr=subprocess.STDOUT)
    try:
        desktop.wait("the menu open", lambda: "OPEN true" in log.read_text(), 30)
        time.sleep(1)
        shot = output / f"{name}.png"
        capture(desktop, shot)
        image = Image.open(shot).convert("RGB")
        window, menu = box(image, CONTENT), box(image, MENU)
        checks[f"{name}: boxes"] = {"window": window, "menu": menu}
        checks[f"{name}: a window and its menu are drawn"] = bool(window and menu)
        said = log.read_text()
        checks[f"{name}: blur_behind is what the desktop does"] = (
            f"BLUR behind={'true' if name in BLURS else 'false'}" in said)
    finally:
        stop(app)


def maximize(desktop, probe, output, checks):
    """popup_probe maximized at its own word: does the app hear it done?"""
    name = desktop.name
    log = output / f"{name}-maximize.log"
    app = subprocess.Popen([str(probe)], env=dict(desktop.env, POPUP_PROBE_MAXIMIZE="1"), stdout=log.open("w"),
                           stderr=subprocess.STDOUT)
    try:
        desktop.wait("the window maximized", lambda: "MAXIMIZED true" in log.read_text(), 15)
        checks[f"{name}: maximized follows the system"] = True
    except TimeoutError:
        checks[f"{name}: maximized follows the system"] = False
    finally:
        stop(app)


def glass(desktop, probe, capture, output, checks, expect):
    """glass_counter over stripes: `expect` blurred or sharp through it."""
    name = desktop.name
    wall = output / "stripes.png"
    stripes(wall)
    backdrop = desktop.start("wallpaper", ["swaybg", "-i", wall, "-m", "fill"])
    time.sleep(1.5)
    bare = output / f"{name}-wallpaper.png"
    capture(desktop, bare)
    app = subprocess.Popen([str(probe)], env=desktop.env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    try:
        shot = output / f"{name}-glass.png"
        before = Image.open(bare).convert("RGB")

        def drawn():
            capture(desktop, shot)
            after = Image.open(shot).convert("RGB")
            return ImageChops.difference(before, after).convert("L").point(lambda v: 255 if v > 24 else 0).getbbox()

        # Its first frame compiles its shaders, in software: seconds.
        desktop.wait("glass_counter drawn", drawn, 30)
        time.sleep(1)
        changed = drawn()
        after = Image.open(shot).convert("RGB")
        # A strip across its lower part, where it draws nothing of its own.
        left, top, right, bottom = changed
        area = (left + 40, bottom - 60, right - 40, bottom - 30)
        share = roughness(after, area) / max(.01, roughness(before, area))
        checks[f"{name}: glass: stripes' roughness kept"] = round(share, 3)
        checks[f"{name}: glass: the desktop behind is {expect}"] = (
            share < BLURRED_BELOW if expect == "blurred" else share > SHARP_ABOVE)
    finally:
        stop(app)
        stop(backdrop)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binaries", type=pathlib.Path, help="the zgx-desktop examples")
    parser.add_argument("--output", type=pathlib.Path, required=True)
    parser.add_argument("--only", action="append", help="a desktop's name; all by default")
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    probe, counter = args.binaries.resolve() / "popup_probe", args.binaries.resolve() / "glass_counter"
    checks, ran, skipped = {}, [], []
    for name, needs, start, capture in DESKTOPS:
        if args.only and name not in args.only:
            continue
        missing = [binary for binary in needs if not shutil.which(binary)]
        if missing:
            skipped.append(f"{name} (no {', '.join(missing)})")
            continue
        desktop = Desktop(name, args.output)
        try:
            start(desktop)
            popup(desktop, probe, capture, args.output, checks)
            if name in ("kwin", "sway") and shutil.which("swaybg"):
                glass(desktop, counter, capture, args.output, checks, "blurred" if name == "kwin" else "sharp")
            if name not in NO_MAXIMIZE:
                maximize(desktop, probe, args.output, checks)
            ran.append(name)
        except (TimeoutError, OSError, AssertionError, StopIteration) as error:
            checks[f"{name}: ran"] = False
            checks[f"{name}: why"] = str(error)
        finally:
            desktop.stop()
    checks["ran"], checks["skipped"] = ran, skipped
    (args.output / "result.json").write_text(json.dumps(checks, indent=2) + "\n")
    failed = [key for key, value in checks.items() if value is False]
    assert not failed, f"failed: {failed}\n{json.dumps(checks, indent=2)}"
    print(f"PASS: a window and its menu on {', '.join(ran) or 'nothing'}; skipped: {', '.join(skipped) or 'none'}")


if __name__ == "__main__":
    main()
