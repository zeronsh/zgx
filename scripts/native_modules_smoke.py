#!/usr/bin/env python3
"""zgx-run loads an app's native modules from its zgx.json before its
scripts (docs/modules.md): a Luau app under zgx-run, in a real window under
Xvfb, calls the typed fixture module (crates/zgx-module/fixture-typed) as
`ui.module("typed")` and by name, and its states are read over the devtools
socket. Then: a library handed over with `--module` is the one loaded, and
`--notice` shows in the banner; a module zgx.json names that isn't built
stops zgx-run before a window opens, naming it; and one whose manifest is
for another interface is shown in the banner, its scripts not run. Last,
`zgx run` with a module's crate: built into the cache and loaded, taken
from the cache the next time, and as its source changes, built again and
the app restarted with a notice. And `zgx build`'s bundle of that app run
by zgx-run: its bytecode and its module's library, checked; a script
changed since the build refused as it is read, with the reason; and one
whose library was changed refused before a window opens."""
import argparse
import json
import os
import pathlib
import shutil
import socket
import struct
import subprocess
import tempfile
import time

ROOT = pathlib.Path(__file__).resolve().parent.parent

APP = """
local typed = ui.module("typed")
ui.state("joined", typed.join("a", "b", "+"))
ui.state("words", ui.call_sync("typed.count_words", "one two three"))
return ui.text { id = "main", "main" }
"""


def ask(path, request):
    with socket.socket(socket.AF_UNIX) as client:
        client.settimeout(15)
        client.connect(str(path))
        client.sendall((json.dumps(request) + "\n").encode())
        answer = b""
        while not answer.endswith(b"\n"):
            chunk = client.recv(1 << 20)
            if not chunk:
                break
            answer += chunk
    return json.loads(answer)


def state(path, name):
    answer = ask(path, {"cmd": "state", "name": name})
    assert answer["ok"], answer
    return answer["text"].split(" = ", 1)[1]


def fixture():
    """The typed fixture module, built: its library's path."""
    built = subprocess.run(
        ["cargo", "build", "--locked", "-p", "zgx-module-fixture-typed", "--message-format=json"],
        cwd=ROOT, capture_output=True, text=True, check=True)
    for line in built.stdout.splitlines():
        message = json.loads(line)
        if message.get("reason") == "compiler-artifact" and message["target"]["name"] == "zgx_module_fixture_typed":
            for file in message["filenames"]:
                if file.endswith(".so"):
                    return pathlib.Path(file)
    raise AssertionError("no library of zgx-module-fixture-typed")


def manifest(json_text):
    """A module manifest's bytes (zgx_module::abi::MANIFEST)."""
    data = json_text.encode()
    total = 0x811C9DC5
    for byte in data:
        total = ((total ^ byte) * 0x01000193) & 0xFFFFFFFF
    return b"zgx-module-json\0" + struct.pack("<IIII", 1, len(data), total, 0) + data + b"\0zgx-module-end\0"


def launch(zgx_run, app, env, *flags):
    process = subprocess.Popen([str(zgx_run), *flags, str(app)], env=env,
                               stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, text=True)
    path = pathlib.Path(env["TMPDIR"]) / f"zgx-inspect-{os.getuid()}" / f"{process.pid}.sock"
    deadline = time.monotonic() + 60
    while not path.exists():
        if process.poll() is not None:
            return process, None
        assert time.monotonic() < deadline, "no devtools socket"
        time.sleep(0.1)
    return process, path


def stop(process):
    process.kill()
    process.wait()


def errors(path):
    answer = ask(path, {"cmd": "errors"})
    assert answer["ok"], answer
    return answer["text"]


MODULE = """
#[zgx_module::module(name = "greet")]
mod greet {
    #[zgx_module::export]
    fn hello(name: &str) -> String {
        format!("GREETING, {name}")
    }
}
"""

GREET_APP = """
ui.state("said", ui.module("greet").hello("you"))
return ui.text { id = "main", "main" }
"""


def zgx_run_app(zgx, app, env):
    """`zgx run <app>`: its process, and stderr as it goes into a file."""
    log = open(app / "run.log", "w")
    process = subprocess.Popen([str(zgx), "run", str(app)], env=env, stdout=subprocess.DEVNULL,
                               stderr=log, text=True)
    return process, log


def socket_of(env, exclude=()):
    """The devtools socket of a zgx-run that serves one (not those in `exclude`)."""
    folder = pathlib.Path(env["TMPDIR"]) / f"zgx-inspect-{os.getuid()}"
    deadline = time.monotonic() + 180
    while time.monotonic() < deadline:
        found = [p for p in folder.glob("*.sock") if p not in exclude] if folder.exists() else []
        for path in found:
            try:
                if ask(path, {"cmd": "state", "name": "said"})["ok"]:
                    return path
            except OSError:
                pass
        time.sleep(0.2)
    raise AssertionError("no zgx-run serving devtools")


def zgx_run_phase(zgx, work, env):
    app = work / "greet-app"
    crate = app / "modules/greet"
    (crate / "src").mkdir(parents=True)
    (crate / "Cargo.toml").write_text(f"""[workspace]
[package]
name = "greet"
version = "0.1.0"
edition = "2024"
[lib]
crate-type = ["cdylib"]
[dependencies]
zgx-module = {{ path = "{ROOT / 'crates/zgx-module'}" }}
""")
    (crate / "src/lib.rs").write_text(MODULE)
    (app / "app.luau").write_text(GREET_APP)
    (app / "zgx.json").write_text(json.dumps({"title": "Greet", "modules": {"greet": "modules/greet"}}))
    env = dict(env, ZGX_CACHE=str(work / "cache"))
    # Built, loaded, answering.
    process, log = zgx_run_app(zgx, app, env)
    try:
        path = socket_of(env)
        assert state(path, "said") == '"GREETING, you"', state(path, "said")
        built = (app / "run.log").read_text()
        assert "module `greet` built in" in built, built
        # Its source changes: built again, the app started again with a
        # notice.
        (crate / "src/lib.rs").write_text(MODULE.replace("GREETING", "HELLO"))
        path = socket_of(env, exclude={path})
        deadline = time.monotonic() + 120
        while state(path, "said") != '"HELLO, you"':
            assert time.monotonic() < deadline, state(path, "said")
            time.sleep(0.2)
        said = (app / "run.log").read_text()
        assert "zgx run: module `greet` changed, restarting" in said, said
        assert "module `greet` changed: restarted" in errors(path), errors(path)
    finally:
        process.kill()
        process.wait()
        log.close()
        # (zgx run's zgx-run: killed with it, by the process group below.)
        subprocess.run(["pkill", "-KILL", "-P", str(process.pid)], check=False)
    # Started again unchanged: the cache's, nothing built.
    (app / "run.log").unlink()
    process, log = zgx_run_app(zgx, app, env)
    try:
        path = socket_of(env, exclude=set())
        assert state(path, "said") == '"HELLO, you"', state(path, "said")
        assert "built in" not in (app / "run.log").read_text(), (app / "run.log").read_text()
    finally:
        process.kill()
        process.wait()
        log.close()
        subprocess.run(["pkill", "-KILL", "-P", str(process.pid)], check=False)


def bundle_phase(zgx, zgx_run, work, env):
    app = work / "greet-app"
    env = dict(env, ZGX_CACHE=str(work / "cache"), ZGX_BUNDLE_TIMING="1")
    out = work / "dist/greet"
    # For release, built once (`zgx run` built it for development), then the
    # cache's.
    for expected in ("module `greet`: built in", "module `greet`: unchanged, from the cache"):
        built = subprocess.run([str(zgx), "build", "--out", str(out), str(app)], env=env,
                               capture_output=True, text=True)
        assert built.returncode == 0, built.stderr
        assert expected in built.stdout, built.stdout
    process, path = launch(zgx_run, out, env)
    assert path, f"zgx-run stopped: {process.stderr.read()}"
    try:
        assert state(path, "said") == '"HELLO, you"', state(path, "said")
    finally:
        stop(process)
    said = process.stderr.read()
    assert "bundle checked in" in said, said
    # A script changed: refused as it is read, with the reason.
    entry = out / "ui/app.luau"
    original = entry.read_bytes()
    entry.write_bytes(original[:-1] + bytes([original[-1] ^ 1]))
    process, path = launch(zgx_run, out, env)
    try:
        assert path, f"zgx-run stopped: {process.stderr.read()}"
        deadline = time.monotonic() + 30
        while "isn't the file its bundle's manifest names" not in (said := errors(path)):
            assert time.monotonic() < deadline, said
            time.sleep(0.1)
    finally:
        stop(process)
    entry.write_bytes(original)
    # Its module's library changed: refused, before any window.
    library = next((out / "modules").glob("*/*"))
    library.write_bytes(library.read_bytes() + b"\0")
    process, path = launch(zgx_run, out, env)
    assert path is None, "zgx-run ran a bundle whose library changed"
    said = process.stderr.read()
    assert "isn't the library its manifest names" in said, said


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("zgx_run", type=pathlib.Path, help="zgx-run, built with its `modules` feature")
    parser.add_argument("--output", type=pathlib.Path, required=True)
    parser.add_argument("--zgx", type=pathlib.Path, help="the zgx tool (default: beside zgx-run)")
    args = parser.parse_args()
    zgx = args.zgx or args.zgx_run.with_name("zgx")
    args.output.mkdir(parents=True, exist_ok=True)
    work = pathlib.Path(tempfile.mkdtemp(dir=args.output)).resolve()
    library = fixture()
    app = work / "app"
    (app / "lib").mkdir(parents=True)
    shutil.copy(library, app / "lib/libtyped.so")
    (app / "app.luau").write_text(APP)
    (app / "zgx.json").write_text(json.dumps({"title": "Modules", "modules": {"typed": "lib/libtyped.so"}}))
    xvfb = subprocess.Popen(["Xvfb", "-r", "-noreset", "-displayfd", "1", "-screen", "0", "1024x768x24", "-ac"],
                            stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True)
    try:
        display = ":" + xvfb.stdout.readline().strip()
        env = dict(os.environ, DISPLAY=display, TMPDIR=str(work), ZGX_INSPECT_SOCKET="1",
                   XDG_RUNTIME_DIR="/tmp", DBUS_SESSION_BUS_ADDRESS="unix:path=/nonexistent")
        env.pop("WAYLAND_DISPLAY", None)

        # zgx.json's module, loaded before the scripts that call it.
        process, path = launch(args.zgx_run, app, env)
        assert path, f"zgx-run stopped: {process.stderr.read()}"
        try:
            assert state(path, "joined") == '"a+b"', state(path, "joined")
            assert state(path, "words") == "3", state(path, "words")
            assert "none" in errors(path).lower() or errors(path).strip() == "", errors(path)
        finally:
            stop(process)

        # `--module` hands over another library for it, and `--notice`
        # shows in the banner.
        given = work / "given/libtyped.so"
        given.parent.mkdir()
        shutil.copy(library, given)
        (app / "lib/libtyped.so").unlink()
        process, path = launch(args.zgx_run, app, env, "--module", f"typed={given}",
                               "--notice", "module `typed` changed: restarted")
        assert path, f"zgx-run stopped: {process.stderr.read()}"
        try:
            assert state(path, "joined") == '"a+b"', state(path, "joined")
            assert "module `typed` changed: restarted" in errors(path), errors(path)
        finally:
            stop(process)

        # zgx.json names a module with no library: stopped before a window,
        # naming it.
        process, path = launch(args.zgx_run, app, env)
        assert path is None, "zgx-run ran without its module's library"
        said = process.stderr.read()
        assert process.returncode != 0 and "module `typed`" in said, (process.returncode, said)

        # A module for another interface: shown, and the scripts don't run.
        (app / "lib/libtyped.so").write_bytes(
            b"\0" * 64 + manifest('{"zgx_module":1,"abi":2,"name":"typed"}') + b"\0" * 64)
        process, path = launch(args.zgx_run, app, env)
        assert path, f"zgx-run stopped: {process.stderr.read()}"
        try:
            shown = errors(path)
            assert "module `typed`" in shown and "another module interface" in shown, shown
            answer = ask(path, {"cmd": "state", "name": "joined"})
            assert not answer["ok"] or "nil" in answer.get("text", ""), answer
        finally:
            stop(process)
        # (The earlier zgx-runs' sockets are gone with them.)
        for old in (work / f"zgx-inspect-{os.getuid()}").glob("*.sock"):
            old.unlink()
        zgx_run_phase(zgx, work, env)
        for old in (work / f"zgx-inspect-{os.getuid()}").glob("*.sock"):
            old.unlink()
        bundle_phase(zgx, args.zgx_run, work, env)
        print("PASS: zgx-run loads zgx.json's module before the scripts that call it, takes one --module "
              "hands over, shows --notice, stops naming one that isn't built, and shows one it refuses; "
              "zgx run builds a module's crate, takes it from the cache when unchanged, and builds it "
              "again and restarts the app with a notice as its source changes; and zgx-run runs zgx build's "
              "bundle of it, refuses a script changed since the build as it is read, and refuses the "
              "bundle before a window once its library changes")
    finally:
        xvfb.terminate()


if __name__ == "__main__":
    main()
