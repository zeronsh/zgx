#!/usr/bin/env python3
"""Build an iOS example, bundle it and run it on a headless simulator.

    python3 scripts/ios/sim.py run -p zgx-desktop --example gallery --shot out.png

Boots a device of its own with `simctl boot` (Simulator.app stays closed, so
nothing takes focus). Results here say nothing about performance, memory or
feel: the simulator draws on the Mac's GPU.
"""

import argparse
import os
import plistlib
import shutil
import subprocess
import sys
import time
from pathlib import Path

DEVICE = "zgx-ios"
DEVICE_TYPE = "com.apple.CoreSimulator.SimDeviceType.iPhone-17-Pro"
TARGET = "aarch64-apple-ios-sim"
ROOT = Path(__file__).resolve().parents[2]


def simctl(*args, check=True, capture=True):
    return subprocess.run(
        ["xcrun", "simctl", *args], check=check, capture_output=capture, text=True
    )


def device() -> str:
    """This script's own device, made once and booted headless."""
    listing = simctl("list", "devices", "available").stdout
    udid = None
    for line in listing.splitlines():
        if line.strip().startswith(DEVICE + " ("):
            udid = line.strip().split("(")[1].split(")")[0]
            break
    if udid is None:
        runtimes = simctl("list", "runtimes", "available").stdout
        runtime = [
            line.split(" - ")[-1].strip()
            for line in runtimes.splitlines()
            if line.startswith("iOS ")
        ][-1]
        udid = simctl("create", DEVICE, DEVICE_TYPE, runtime).stdout.strip()
    simctl("bootstatus", udid, "-b")
    return udid


def bundle(binary: Path, name: str, resources: list[Path]) -> Path:
    app = ROOT / "target" / "ios" / f"{name}.app"
    shutil.rmtree(app, ignore_errors=True)
    app.mkdir(parents=True)
    shutil.copy2(binary, app / name)
    for resource in resources:
        if resource.is_dir():
            shutil.copytree(resource, app / resource.name, symlinks=True)
        else:
            shutil.copy2(resource, app / resource.name)
    identifier = "sh.zeron.zgx.ios." + name.replace("_", "-")
    with open(app / "Info.plist", "wb") as f:
        plistlib.dump(
            {
                "CFBundleIdentifier": identifier,
                "CFBundleExecutable": name,
                "CFBundleName": name,
                "CFBundlePackageType": "APPL",
                "CFBundleShortVersionString": "0.1",
                "CFBundleVersion": "1",
                "MinimumOSVersion": "26.0",
                "UIDeviceFamily": [1, 2],
                "UILaunchScreen": {},
                "UISupportedInterfaceOrientations": [
                    "UIInterfaceOrientationPortrait",
                    "UIInterfaceOrientationLandscapeLeft",
                    "UIInterfaceOrientationLandscapeRight",
                ],
            },
            f,
        )
    return app


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("command", choices=["run", "shutdown"])
    parser.add_argument("-p", "--package", default="zgx-desktop")
    parser.add_argument("--example")
    parser.add_argument("--bin")
    parser.add_argument("--release", action="store_true")
    parser.add_argument("--features", default="")
    parser.add_argument("--resource", action="append", default=[], type=Path)
    parser.add_argument("--env", action="append", default=[], help="NAME=value")
    parser.add_argument("--wait", type=float, default=6.0)
    parser.add_argument("--shot", type=Path)
    parser.add_argument("--orientation")
    parser.add_argument("--keep", action="store_true", help="leave the app running")
    args = parser.parse_args()
    if args.command == "shutdown":
        simctl("shutdown", device(), check=False)
        return 0

    name = args.example or args.bin
    environment = dict(os.environ, CARGO_PROFILE_DEV_DEBUG="0")
    # Xcode's python3 is started through xcrun, which points SDKROOT at macOS's.
    environment.pop("SDKROOT", None)
    build = ["cargo", "build", "-p", args.package, "--target", TARGET, "--locked"]
    build += ["--example", args.example] if args.example else ["--bin", args.bin]
    if args.release:
        build.append("--release")
    if args.features:
        build += ["--features", args.features]
    subprocess.run(build, check=True, cwd=ROOT, env=environment)
    profile = "release" if args.release else "debug"
    directory = ROOT / "target" / TARGET / profile
    binary = directory / "examples" / name if args.example else directory / name
    app = bundle(binary, name, args.resource)

    udid = device()
    identifier = "sh.zeron.zgx.ios." + name.replace("_", "-")
    simctl("terminate", udid, identifier, check=False)
    simctl("install", udid, str(app))
    log = ROOT / "target" / "ios" / f"{name}.log"
    log.unlink(missing_ok=True)
    launch = dict(environment)
    for pair in args.env:
        key, value = pair.split("=", 1)
        launch["SIMCTL_CHILD_" + key] = value
    launched = subprocess.run(
        ["xcrun", "simctl", "launch", f"--stdout={log}", f"--stderr={log}", udid, identifier],
        env=launch,
        capture_output=True,
        text=True,
    )
    print(launched.stdout.strip(), launched.stderr.strip())
    time.sleep(args.wait)
    alive = simctl("spawn", udid, "launchctl", "list", check=False).stdout
    print("running:", identifier in alive)
    if args.shot:
        args.shot.parent.mkdir(parents=True, exist_ok=True)
        simctl("io", udid, "screenshot", str(args.shot.resolve()))
        print("shot:", args.shot)
    if log.exists():
        print(log.read_text()[-3000:])
    if not args.keep:
        simctl("terminate", udid, identifier, check=False)
    return launched.returncode


if __name__ == "__main__":
    sys.exit(main())
