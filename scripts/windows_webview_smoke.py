#!/usr/bin/env python3
"""A WebView2 page in zgx on Windows (the `web_view` example, built with the
`webview2` feature), driven by SendInput: the page loads and its title reaches
`WebViewState`; a click in the page reaches it (its message comes back); zgx
content over the page takes clicks and the keyboard back from it; script
evaluation changes the page; the menu popover draws above the page and takes
its clicks; the wheel scrolls the page; and after a resize the page still
fills its node, edge to edge.

    cargo build -p zgx-desktop --features webview2 --example web_view
    python scripts/windows_webview_smoke.py target/debug/examples/web_view.exe

`--no-foreground` sends no input, for a desktop another window may hold:
the example's `--control` commands on stdin find in the page (and
WebView2's browser keys read back off, `ZGX_WEB_SETTINGS`), and have the
page's own script start its download, with no handler, one cancelling and
one saving. Then the real Ctrl+F, a case of its own: BLOCKED, not failed,
when a system overlay (a notification, Start) holds the foreground for 60 s.
`--only trust` runs only the page loading and the certificate and
credentials cases below (as CI's Windows job does).
`ZGX_WEB_SETTINGS=1` has the app also print each permission WebView2 asks
for and each download it starts (`WEB_PERMISSION`, `WEB_DOWNLOAD_STARTING`).
A page's own click that starts no download fails the run; the app opening
the file itself (`open-download`) then only lets the handler's checks run.
Then certificates and credentials (`trust_servers`): with no handler, or a
responder dropped, a server whose certificate isn't trusted loads nothing
and a credentials challenge gets the refusal; proceeding loads that one
page, and the next load from the server, in the page or in a second page on
the same profile, is asked again, even the second page's load started the
moment the app proceeded.
"""
import argparse
import os
import pathlib
import subprocess
import ctypes
import tempfile
import threading
import time

import trust_servers
import windows_input as win

BACKGROUND = (0x14, 0x15, 0x19)


def wait_for(check, what, timeout=10.0):
    deadline = time.monotonic() + timeout
    while not check():
        if time.monotonic() > deadline:
            raise AssertionError(f"timed out waiting for {what}")
        time.sleep(0.05)


def near(a, b, tolerance=12):
    return all(abs(x - y) <= tolerance for x, y in zip(a, b))


def white_pixels(capture, box):
    """How many near-white pixels `capture` has in `box` (left, top, right, bottom)."""
    left, top, right, bottom = box
    return sum(
        1
        for y in range(top, bottom, 2)
        for x in range(left, right, 2)
        if min(win.pixel(capture, x, y)) > 235
    )


def differs(a, b, box):
    """The share of `box`'s pixels that differ between captures `a` and `b`."""
    left, top, right, bottom = box
    points = [(x, y) for y in range(top, bottom, 3) for x in range(left, right, 3)]
    changed = sum(1 for x, y in points if not near(win.pixel(a, x, y), win.pixel(b, x, y)))
    return changed / len(points)


def overlay_in_front(pid):
    """The class and title of a system overlay (a notification, Start,
    search: a `Windows.UI.Core.CoreWindow`) of another process holding the
    foreground, or None."""
    user32 = ctypes.windll.user32
    hwnd = user32.GetForegroundWindow()
    if not hwnd or hwnd in win.windows_of(pid):
        return None
    name = ctypes.create_unicode_buffer(256)
    user32.GetClassNameW(hwnd, name, 256)
    if name.value != "Windows.UI.Core.CoreWindow":
        return None
    return f"{name.value} {win.title(hwnd)!r}"


def controlled(proc, window, lines, heard, save_dir):
    """The cases `--no-foreground` drives through the example's stdin."""
    def send(command):
        proc.stdin.write(command + "\n")
        proc.stdin.flush()

    wait_for(lambda: heard("WEB_SETTINGS browser_keys="), "WebView2's settings read back")
    assert heard("WEB_SETTINGS browser_keys=false"), [l for l in lines if l.startswith("WEB_SETTINGS")]
    print("ok: WebView2's browser keys read back off")

    send("find Hello")
    wait_for(lambda: heard("FIND Hello "), "the app's find")
    assert heard("FIND Hello found=true"), [l for l in lines if l.startswith("FIND")]
    send("find nowhere-in-the-page")
    wait_for(lambda: heard("FIND nowhere-in-the-page "), "the app's second find")
    assert heard("FIND nowhere-in-the-page found=false"), [l for l in lines if l.startswith("FIND")]
    print("ok: the app's find finds the heading's word, and not a word the page hasn't")

    downloads = pathlib.Path(os.environ["USERPROFILE"], "Downloads")
    listing = lambda: set(os.listdir(downloads)) if downloads.is_dir() else set()
    before = listing()
    count = lambda prefix: sum(line.startswith(prefix) for line in lines)
    # Cases whose download only the app's own open started: the page's
    # click starting none is the bug a user hits, so the run fails at the
    # end, after the handler's checks ran on the app's download.
    unclicked = []

    def download(case, fresh=True):
        """Start the page's download: its script clicks its link, or, if
        WebView2 starts none from that within 5 s (a click with no gesture
        behind it), the app opens the file itself. Which, as `case` ran.
        `fresh`: the page reloaded first, as Chromium holds back a page's
        further downloads (its count starts again with a navigation)."""
        if fresh:
            loaded = count("FINISHED ")
            send("reload")
            wait_for(lambda: count("FINISHED ") > loaded, f"the page to reload ({case})", 20)
        started = count("WEB_DOWNLOAD_STARTING")
        send("save-link")
        deadline = time.monotonic() + 5
        while count("WEB_DOWNLOAD_STARTING") == started and time.monotonic() < deadline:
            time.sleep(0.05)
        how = "the page's click"
        if count("WEB_DOWNLOAD_STARTING") == started:
            how = "the app's open (the page's click started none)"
            unclicked.append(case)
            send("open-download")
            wait_for(lambda: count("WEB_DOWNLOAD_STARTING") > started, f"WebView2 to start the download ({case})")
        print(f"note: {case}: the download came from {how}")

    # No handler: WebView2 starts the download, and zgx cancels it.
    download("no handler", fresh=False)
    time.sleep(2.0)
    assert not heard("DOWNLOAD "), "a download reached a handler the app never set"
    assert listing() == before and not any(save_dir.iterdir()), "a download with no handler was saved"
    print("ok: with no handler the page's download saves nothing")
    send("downloads cancel")
    wait_for(lambda: heard("DOWNLOADS cancelled"), "the cancelling handler")
    download("cancelled")
    wait_for(lambda: heard("DOWNLOAD_CANCELLED "), "the handler to hear the download")
    time.sleep(2.0)
    assert listing() == before and not any(save_dir.iterdir()), "a cancelled download was saved"
    print("ok: a download the handler cancels saves nothing")
    send("downloads save")
    wait_for(lambda: heard("DOWNLOADS on"), "the saving handler")
    asked = count("DOWNLOAD ")
    download("saved")
    wait_for(lambda: count("DOWNLOAD ") > asked, "the handler to hear the download")
    name = [line for line in lines if line.startswith("DOWNLOAD ")][-1].split(" ")[1]
    wait_for(lambda: heard("DOWNLOAD_STATUS Finished"), "the app's download to finish", 20)
    saved = save_dir / name
    assert saved.read_text() == "saved by the app", saved
    assert listing() == before, "the app's download also landed in Downloads"
    print(f"ok: the handler's download saves where it says ({name}), and only there")
    return unclicked


def trust_cases(proc, lines, heard):
    """Certificate errors and credentials, through the example's stdin."""
    def send(command):
        proc.stdin.write(command + "\n")
        proc.stdin.flush()

    (tls_server, tls), (_, auth) = trust_servers.https(), trust_servers.basic_auth()

    def explain(what):
        """`what`, with the paths the TLS server served and the app's last lines."""
        tail = "\n  ".join(lines[-30:])
        return f"{what}\nthe TLS server served {tls_server.paths}\nthe app's last lines:\n  {tail}"
    count = lambda prefix: sum(line.startswith(prefix) for line in lines)
    loaded = lambda prefix, path: any(line.startswith(prefix) and f'"{path}"' in line for line in lines)

    def refused(path, what):
        time.sleep(3.0)
        assert not loaded("MESSAGE ", path), f"{what}: the page loaded past the certificate"

    send(f"load {tls}/none")
    refused("/none", "no handler")
    assert not heard("CERT "), "a certificate error reached a handler the app never set"
    print("ok: with no certificate handler the untrusted server loads nothing")
    send("cert drop")
    wait_for(lambda: heard("CERTS drop"), "the dropping handler")
    send(f"load {tls}/drop")
    wait_for(lambda: heard("CERT 127.0.0.1 can_proceed=true"), "the handler to hear the certificate error", 20)
    refused("/drop", "a dropped responder")
    print("ok: a certificate responder dropped unanswered loads nothing")
    send("cert race")
    wait_for(lambda: heard("CERTS race"), "the proceeding handler")
    asked, second_asked = count("CERT "), count("SECOND_CERT ")
    send(f"load {tls}/proceed")
    wait_for(lambda: loaded("MESSAGE ", "/proceed"), "the page the app proceeded to", 20)
    wait_for(lambda: count("SECOND_CERT ") > second_asked,
             "the second page, loading the same server as the app proceeded, to be asked", 20)
    time.sleep(2.0)
    assert not loaded("SECOND_MESSAGE ", "/race"), "the second page loaded on the first page's proceed"
    assert count("CERT ") == asked + 1, [line for line in lines if line.startswith("CERT ")]
    print("ok: proceeding loads that page; a second page on the profile, loading the server at once, is still asked")
    send("cert cancel")
    wait_for(lambda: heard("CERTS cancel"), "the cancelling handler")
    asked, second_asked = count("CERT "), count("SECOND_CERT ")
    send(f"load {tls}/again")
    try:
        wait_for(lambda: count("CERT ") > asked or loaded("MESSAGE ", "/again"), "", 20)
    except AssertionError:
        raise AssertionError(explain("the next load from the server was neither asked nor loaded")) from None
    assert count("CERT ") > asked, explain("the next load from the server loaded without the app asked")
    refused("/again", "the next load, cancelled")
    send(f"load2 {tls}/later")
    wait_for(lambda: count("SECOND_CERT ") > second_asked, "the second page's later load to be asked", 20)
    assert not loaded("SECOND_MESSAGE ", "/later"), "the second page loaded past the certificate"
    print("ok: proceeding was for that load: the next ones, in either page, are asked again")

    send(f"load {auth}/none")
    wait_for(lambda: any(line.startswith("MESSAGE ") and "refused" in line for line in lines),
             "the server's refusal with no credentials handler", 20)
    assert not heard("AUTH "), "a challenge reached a handler the app never set"
    # Dropped before answered: once answered, an engine may send the
    # credentials to that server again for the session unasked.
    refusals = sum(line.startswith("MESSAGE ") and "refused" in line for line in lines)
    send("auth drop")
    wait_for(lambda: heard("AUTHS drop"), "the dropping handler")
    send(f"load {auth}/dropped")
    wait_for(lambda: sum(line.startswith("MESSAGE ") and "refused" in line for line in lines) > refusals,
             "a dropped responder to leave the server's refusal", 20)
    assert heard("AUTH zgx basic"), [line for line in lines if line.startswith("AUTH ")]
    send("auth answer")
    wait_for(lambda: heard("AUTHS answer"), "the answering handler")
    send(f"load {auth}/answered")
    wait_for(lambda: any(line.startswith("MESSAGE ") and '"ok"' in line for line in lines),
             "the app's credentials to load the page", 20)
    print("ok: credentials: none without a handler or with a dropped responder; the app's answer loads the page")


def key_case(proc, window, lines, at):
    """Ctrl+F from the keyboard, the one case that needs the foreground."""
    deadline = time.monotonic() + 60
    while (held := overlay_in_front(proc.pid)) and time.monotonic() < deadline:
        time.sleep(0.5)
    if held:
        print(f"BLOCKED: Ctrl+F from the keyboard: {held} held the foreground for 60 s; no input sent")
        return
    found = lambda: sum(line.startswith("FIND Hello found=true") for line in lines)
    before = found()
    win.activate(window)
    win.click(window, *at(150, 298))
    win.key("ctrl+f")
    wait_for(lambda: found() > before, "the app's find after Ctrl+F in the page")
    print("ok: Ctrl+F in the page reaches the app, and its find finds the heading")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=pathlib.Path)
    parser.add_argument("--shots", type=pathlib.Path, help="save captures here")
    parser.add_argument("--no-foreground", action="store_true",
                        help="send no input: drive the page through the example's stdin")
    parser.add_argument("--only", choices=["trust"],
                        help="with --no-foreground, only the page loading and these cases (CI's Windows job)")
    args = parser.parse_args()
    lines = []
    save_dir = pathlib.Path(tempfile.mkdtemp(prefix="zgx-downloads-"))
    command = [str(args.binary.resolve())] + (["--control"] if args.no_foreground else [])
    proc = subprocess.Popen(
        command, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True,
        stdin=subprocess.PIPE if args.no_foreground else None,
        env=dict(os.environ, ZGX_EXAMPLE_SAVE_DIR=str(save_dir), ZGX_WEB_SETTINGS="1"),
    )
    threading.Thread(target=lambda: lines.extend(proc.stdout), daemon=True).start()
    heard = lambda prefix: any(line.startswith(prefix) for line in lines)

    def shot(name):
        capture = win.capture(window)
        if args.shots:
            args.shots.mkdir(parents=True, exist_ok=True)
            win.save_png(capture, args.shots / f"{name}.png")
        return capture

    try:
        window = win.wait_window(proc)
        scale = win.dpi(window) / 96
        if args.no_foreground:
            at = lambda x, y: (round(x * scale), round(y * scale))
            wait_for(lambda: heard("FINISHED https://zgx.example/"), "the page to load", 30)
            wait_for(lambda: heard("TITLE zgx web view"), "the page's title")
            print("ok: the page loaded and its title reached the state")
            if args.only == "trust":
                trust_cases(proc, lines, heard)
                print("webview smoke (no foreground, trust) passed")
                return
            unclicked = controlled(proc, window, lines, heard, save_dir)
            trust_cases(proc, lines, heard)
            key_case(proc, window, lines, at)
            if unclicked:
                raise AssertionError(
                    "the page's own click on its download link started no download in WebView2 "
                    f"({', '.join(unclicked)}); the handler's checks ran on the app's own open")
            print("webview smoke (no foreground) passed")
            return
        win.activate(window)
        at = lambda x, y: (round(x * scale), round(y * scale))
        box = lambda l, t, r, b: (*at(l, t), *at(r, b))

        wait_for(lambda: heard("FINISHED https://zgx.example/"), "the page to load", 30)
        wait_for(lambda: heard("TITLE zgx web view"), "the page's title")
        time.sleep(0.5)
        print("ok: the page loaded and its title reached the state")

        loaded = shot("loaded")
        assert white_pixels(loaded, box(180, 110, 620, 170)) > 200, "the page's heading shows"
        print("ok: the page shows in its node")

        win.click(window, *at(150, 298))
        wait_for(lambda: heard("MESSAGE"), "the page's message after a click in it")
        print("ok: a click in the page reaches it")

        # Ctrl+F in the page: WebView2's own find key is off, so the page
        # leaves it to the app, whose binding finds the heading's word.
        win.key("ctrl+f")
        wait_for(lambda: heard("FIND Hello found=true"), "the app's find after Ctrl+F in the page")
        print("ok: Ctrl+F in the page reaches the app, and its find finds the heading")

        # Downloads are the app's: with no handler a download link saves
        # nothing in the user's Downloads; after Ctrl+D (the app's handler)
        # it saves in the folder the app says, and only there.
        downloads = pathlib.Path(os.environ["USERPROFILE"], "Downloads", "zgx-example.txt")
        before = downloads.exists()
        win.click(window, *at(740, 348))
        time.sleep(2.0)
        assert downloads.exists() == before, "a download with no handler landed in Downloads"
        win.key("ctrl+d")
        wait_for(lambda: heard("DOWNLOADS on"), "the app's download handler")
        win.click(window, *at(740, 348))
        wait_for(lambda: heard("DOWNLOAD_STATUS Finished"), "the app's download to finish", 20)
        saved = save_dir / "zgx-example.txt"
        assert saved.read_text() == "saved by the app", saved
        assert downloads.exists() == before, "the app's download also landed in Downloads"
        print("ok: a download with no handler saves nothing; the app's handler saves it where it says")

        # zgx's composer over the page: the click and the keyboard are zgx's.
        win.click(window, *at(400, 700))
        time.sleep(0.3)
        win.type_text("Hi")
        time.sleep(0.2)
        win.click(window, *at(780, 700))
        wait_for(
            lambda: white_pixels(shot("evaluated"), box(180, 110, 620, 170)) < 40,
            "the page's heading to change (Send to page)",
        )
        print("ok: zgx content over the page takes clicks and keys; script runs in the page")

        # The menu shows in a window of its own: over the page, and past the
        # window's right edge where the screen has room (else inside it).
        x, y = win.to_screen(window, *at(860, 60))
        strip = lambda: win.capture_rect(x, y, round(420 * scale), round(120 * scale))
        before = strip()
        win.click(window, *at(1060, 24))
        menu = (0, 0, round(420 * scale), round(120 * scale))
        wait_for(lambda: win.popups_of(proc.pid) and differs(before, strip(), menu) > 0.15,
                 "the menu over the page")
        if args.shots:
            win.save_png(strip(), args.shots / "menu.png")
        print("ok: the popover draws above the page")
        # Zoom to 125%, where the menu lies either way: zgx's, over the page.
        win.click(window, *at(1070, 138))
        wait_for(lambda: not win.popups_of(proc.pid), "the menu to close (its click was zgx's)")
        print("ok: a click on the popover over the page is zgx's")

        before = shot("before-scroll")
        win.wheel(window, *at(500, 400), -3)
        wait_for(lambda: differs(before, shot("scrolled"), box(40, 200, 1000, 600)) > 0.2,
                 "the page to scroll")
        print("ok: the wheel over the page scrolls it")

        win.resize_client(window, *at(800, 600))
        time.sleep(1.0)
        resized = shot("resized")
        width, height = win.client_size(window)
        # The page's node is inset 12 points from the window's right and bottom.
        background = lambda point: near(win.pixel(resized, *point), BACKGROUND, 4)
        right = next(x for x in range(width - 1, 0, -1) if not background((x, height // 2)))
        bottom = next(y for y in range(height - 1, 0, -1) if not background((width // 3, y)))
        expected = (width - round(12 * scale) - 1, height - round(12 * scale) - 1)
        assert abs(right - expected[0]) <= 2 and abs(bottom - expected[1]) <= 2, (
            (right, bottom), expected)
        print("ok: after a resize the page fills its node, edge to edge")
        print("webview smoke passed")
    finally:
        if proc.poll() is None:
            try:
                win.close(window)
                proc.wait(5)
            except Exception:
                proc.kill()


if __name__ == "__main__":
    main()
