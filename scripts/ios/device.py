#!/usr/bin/env python3
"""A zgx app on a real iPhone, in one command. See docs/ios-device.md.

    python3 scripts/ios/device.py check
    python3 scripts/ios/device.py run --example glass
    python3 scripts/ios/device.py run -p zgx-run --example ios_app --resource crates/zgx-script/examples/ios_settings
    python3 scripts/ios/device.py measure

Signing can raise a Keychain dialog on the Mac the first time: every signing
and device call has a timeout, so a waiting prompt ends the step with a message
and never hangs. Nothing here answers a dialog.
"""

import argparse
import json
import os
import plistlib
import re
import shutil
import subprocess
import sys
import tempfile
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
TARGET = "aarch64-apple-ios"
BUNDLE = os.environ.get("ZGX_IOS_BUNDLE", "dev.zgx.ios.app")
OUT = ROOT / "target" / "ios-device"
PROFILES = [
    Path.home() / "Library/Developer/Xcode/UserData/Provisioning Profiles",
    Path.home() / "Library/MobileDevice/Provisioning Profiles",
]
# The scenes `measure` runs: what each mode of the ios_cost example shows.
SCENES = [
    ("idle list, no glass", {"COST_MODE": "plain", "COST_STILL": "1"}),
    ("scrolling, no glass", {"COST_MODE": "plain"}),
    ("idle list, 1 glass", {"COST_MODE": "plain", "COST_GLASS": "1", "COST_STILL": "1"}),
    ("scrolling, 1 glass", {"COST_MODE": "plain", "COST_GLASS": "1"}),
    ("scrolling, 4 glass", {"COST_MODE": "plain", "COST_GLASS": "4"}),
    ("scrolling, 4 glass with text over", {"COST_MODE": "plain", "COST_GLASS": "4", "COST_OVER": "1"}),
    ("scrolling, 1 plane over glass", {"COST_MODE": "plain", "COST_LEVELS": "1"}),
    ("scrolling, 2 planes over glass", {"COST_MODE": "plain", "COST_LEVELS": "2"}),
    ("scrolling, 3 planes over glass", {"COST_MODE": "plain", "COST_LEVELS": "3"}),
    ("scrolling, drawn switches", {"COST_MODE": "drawn"}),
    ("scrolling, hosted switches", {"COST_MODE": "hosted"}),
]


def run(args, timeout, **kwargs):
    try:
        return subprocess.run(args, capture_output=True, text=True, timeout=timeout, **kwargs)
    except subprocess.TimeoutExpired:
        sys.exit(
            f"Timed out after {timeout} s: {args[0]} {args[1] if len(args) > 1 else ''}.\n"
            "If a dialog is waiting on the Mac, it is the person's to answer: run this again after."
        )


def devices():
    with tempfile.NamedTemporaryFile(suffix=".json") as out:
        run(["xcrun", "devicectl", "list", "devices", "--json-output", out.name, "--quiet"], 30)
        listing = json.load(open(out.name))
    found = []
    for device in listing.get("result", {}).get("devices", []):
        props = device.get("hardwareProperties", {})
        connection = device.get("connectionProperties", {})
        if props.get("deviceType") == "iPhone" or props.get("platform") == "iOS":
            found.append(
                {
                    "id": device.get("identifier"),
                    "udid": props.get("udid"),
                    "name": device.get("deviceProperties", {}).get("name"),
                    "model": props.get("marketingName"),
                    "os": device.get("deviceProperties", {}).get("osVersionNumber"),
                    "tunnel": connection.get("tunnelState"),
                    "paired": connection.get("pairingState"),
                }
            )
    return found


def pick_device(wanted):
    found = devices()
    if wanted:
        found = [d for d in found if wanted in (d["id"], d["udid"], d["name"])]
    live = [d for d in found if d["tunnel"] == "connected"] or [d for d in found if d["paired"] == "paired"]
    if not live:
        sys.exit("No paired iPhone to be found. Plug it in, unlock it, and trust this Mac on the phone.")
    return live[0]


def identities():
    out = run(["security", "find-identity", "-v", "-p", "codesigning"], 20).stdout
    return re.findall(r'\d+\) ([0-9A-F]{40}) "([^"]+)"', out)


def profiles(udid=None):
    """Provisioning profiles on this Mac that cover BUNDLE (and `udid`, if given)."""
    found = []
    for folder in PROFILES:
        for path in folder.glob("*provision*") if folder.exists() else []:
            out = run(["security", "cms", "-D", "-i", str(path)], 20)
            if out.returncode != 0:
                continue
            info = plistlib.loads(out.stdout.encode())
            app_id = info.get("Entitlements", {}).get("application-identifier", "")
            bundle = app_id.split(".", 1)[-1]
            covers = bundle == BUNDLE or (bundle.endswith("*") and BUNDLE.startswith(bundle[:-1]))
            live = info.get("ExpirationDate") is None or info["ExpirationDate"].timestamp() > time.time()
            on_device = udid is None or info.get("ProvisionsAllDevices") or udid in info.get("ProvisionedDevices", [])
            if covers and live and on_device:
                found.append((path, info))
    # The most specific first.
    return sorted(found, key=lambda item: item[1]["Entitlements"]["application-identifier"].endswith("*"))


def setup():
    """Have Xcode's tools make a provisioning profile for BUNDLE, once: it builds
    an empty app with automatic signing, which signs and so may raise a dialog."""
    if not shutil.which("xcodegen"):
        sys.exit("Needs xcodegen (brew install xcodegen), or make the profile in Xcode: docs/ios-device.md.")
    names = [name for _, name in identities()]
    if not names:
        sys.exit("No Apple Development identity on this Mac: sign in to Xcode, Settings, Accounts.")
    subject = run(["security", "find-certificate", "-c", names[0], "-p"], 20).stdout
    text = subprocess.run(["openssl", "x509", "-noout", "-subject"], input=subject, capture_output=True, text=True).stdout
    team = re.search(r"OU\s*=\s*([A-Z0-9]{10})", text)
    if not team:
        sys.exit("Couldn't read the team from the signing certificate.")
    folder = OUT / "bootstrap"
    shutil.rmtree(folder, ignore_errors=True)
    (folder / "app").mkdir(parents=True)
    (folder / "app" / "main.swift").write_text("import UIKit\nUIApplicationMain(CommandLine.argc, CommandLine.unsafeArgv, nil, nil)\n")
    (folder / "project.yml").write_text(
        f"""name: ZgxBootstrap
options:
  bundleIdPrefix: dev.zgx
targets:
  ZgxBootstrap:
    type: application
    platform: iOS
    deploymentTarget: "26.0"
    sources: [app]
    settings:
      PRODUCT_BUNDLE_IDENTIFIER: {BUNDLE}
      DEVELOPMENT_TEAM: {team.group(1)}
      CODE_SIGN_STYLE: Automatic
      GENERATE_INFOPLIST_FILE: YES
"""
    )
    run(["xcodegen", "generate"], 60, cwd=folder)
    out = run(
        ["xcodebuild", "-project", "ZgxBootstrap.xcodeproj", "-scheme", "ZgxBootstrap", "-destination",
         "generic/platform=iOS", "-allowProvisioningUpdates", "-allowProvisioningDeviceRegistration", "build"],
        300,
        cwd=folder,
    )
    if out.returncode != 0:
        sys.exit("xcodebuild failed:\n" + out.stdout[-1500:] + out.stderr[-500:])
    print("profiles now:", [info.get("Name") for _, info in profiles()])
    return 0


def build(args):
    environment = {k: v for k, v in os.environ.items() if k != "SDKROOT"}
    command = ["cargo", "build", "-p", args.package, "--example", args.example, "--target", TARGET, "--locked", "--release"]
    if subprocess.run(command, cwd=ROOT, env=environment).returncode != 0:
        sys.exit("The build failed.")
    return ROOT / "target" / TARGET / "release" / "examples" / args.example


def bundle_app(binary, resources):
    app = OUT / "zgx.app"
    shutil.rmtree(app, ignore_errors=True)
    app.mkdir(parents=True)
    shutil.copy2(binary, app / "zgx")
    for resource in resources:
        shutil.copytree(resource, app / resource.name, symlinks=True)
    with open(app / "Info.plist", "wb") as f:
        plistlib.dump(
            {
                "CFBundleIdentifier": BUNDLE,
                "CFBundleExecutable": "zgx",
                "CFBundleName": "zgx",
                "CFBundleDisplayName": "zgx",
                "CFBundlePackageType": "APPL",
                "CFBundleShortVersionString": "0.1",
                "CFBundleVersion": str(int(time.time())),
                "MinimumOSVersion": "26.0",
                "UIDeviceFamily": [1, 2],
                "UILaunchScreen": {},
                "UIRequiredDeviceCapabilities": ["arm64", "metal"],
                "UISupportedInterfaceOrientations": [
                    "UIInterfaceOrientationPortrait",
                    "UIInterfaceOrientationLandscapeLeft",
                    "UIInterfaceOrientationLandscapeRight",
                ],
                "DTPlatformName": "iphoneos",
                # Without this an iPhone's display link stays at 60 Hz.
                "CADisableMinimumFrameDurationOnPhone": True,
            },
            f,
        )
    return app


def sign(app, device):
    chosen = profiles(device["udid"])
    if not chosen:
        sys.exit(
            f"No provisioning profile on this Mac covers {BUNDLE} and this phone.\n"
            "See docs/ios-device.md, 'One setup step'."
        )
    path, info = chosen[0]
    shutil.copy2(path, app / "embedded.mobileprovision")
    team = info["TeamIdentifier"][0]
    entitlements = OUT / "entitlements.plist"
    with open(entitlements, "wb") as f:
        wanted = dict(info["Entitlements"])
        wanted["application-identifier"] = f"{team}.{BUNDLE}"
        plistlib.dump(wanted, f)
    certificates = {}
    for certificate in info.get("DeveloperCertificates", []):
        out = subprocess.run(["shasum", "-a", "1"], input=certificate, capture_output=True)
        certificates[out.stdout.split()[0].decode().upper()] = True
    usable = [sha for sha, _ in identities() if sha in certificates]
    if not usable:
        sys.exit("No signing identity on this Mac matches the profile's certificates. See docs/ios-device.md.")
    out = run(
        ["codesign", "--force", "--timestamp=none", "--sign", usable[0], "--entitlements", str(entitlements), str(app)],
        60,
    )
    if out.returncode != 0:
        sys.exit(f"codesign failed:\n{out.stderr}")


def install(app, device):
    out = run(["xcrun", "devicectl", "device", "install", "app", "--device", device["id"], str(app)], 180)
    if out.returncode != 0:
        sys.exit(f"Install failed:\n{out.stdout}\n{out.stderr}")


def launch(device, environment, console=False):
    command = [
        "xcrun", "devicectl", "device", "process", "launch", "--terminate-existing",
        "--device", device["id"],
    ]
    if environment:
        command += ["--environment-variables", json.dumps(environment)]
    if console:
        command.append("--console")
    command.append(BUNDLE)
    if console:
        return subprocess.run(command)
    out = run(command, 60)
    if out.returncode != 0:
        sys.exit(f"Launch failed (a phone must be unlocked):\n{out.stdout}\n{out.stderr}")


def fetch(device, destination):
    out = run(
        [
            "xcrun", "devicectl", "device", "copy", "from",
            "--device", device["id"],
            "--domain-type", "appDataContainer",
            "--domain-identifier", BUNDLE,
            "--source", "Documents/zgx-metrics.jsonl",
            "--destination", str(destination),
        ],
        60,
    )
    return out.returncode == 0


def measure(args, device):
    args.package, args.example = "zgx-desktop", "ios_cost"
    app = bundle_app(build(args), [])
    sign(app, device)
    results = []
    window = max(args.seconds / 3, 2)
    for name, environment in SCENES:
        # Installed anew, so each scene's log is its own.
        run(["xcrun", "devicectl", "device", "uninstall", "app", "--device", device["id"], BUNDLE], 60)
        install(app, device)
        print(f"{name} ({args.seconds:.0f} s)", flush=True)
        launch(device, dict(environment, ZGX_IOS_METRICS=str(window), ZGX_IOS_METRICS_PASSIVE="1", ZGX_IOS_SCENE=name))
        time.sleep(args.seconds + window + 3)
        local = OUT / "metrics.jsonl"
        local.unlink(missing_ok=True)
        if fetch(device, local):
            rows = [json.loads(line) for line in local.read_text().splitlines() if line.strip()]
            results.extend(rows[1:] or rows)
    run(["xcrun", "devicectl", "device", "process", "terminate", "--device", device["id"], "--pid", "0"], 10)
    keys = ["cpu_pct", "gpu_app_pct", "gpu_ms_per_buffer", "gap_p50_ms", "gap_p99_ms", "gap_max_ms", "memory_mb", "thermal"]
    print(f"\n{'scene':38}" + "".join(f"{k:>18}" for k in keys))
    for name, _ in SCENES:
        rows = [row for row in results if row["scene"] == name]
        if not rows:
            print(f"{name:38}no data")
            continue
        def middle(key):
            values = sorted(row[key] for row in rows)
            return values[len(values) // 2]
        print(f"{name:38}" + "".join(f"{middle(k):18.2f}" for k in keys))
    report = OUT / f"measure-{int(time.time())}.json"
    report.write_text(json.dumps(results, indent=1))
    print(f"\nEvery window: {report}")
    print("The compositor's GPU share, glass's above all, needs Instruments: docs/ios-device.md.")


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("command", choices=["check", "setup", "run", "measure"])
    parser.add_argument("-p", "--package", default="zgx-desktop")
    parser.add_argument("--example", default="glass")
    parser.add_argument("--resource", action="append", default=[], type=Path)
    parser.add_argument("--env", action="append", default=[], help="NAME=value")
    parser.add_argument("--device", help="a name or identifier, if several are paired")
    parser.add_argument("--seconds", type=float, default=30.0, help="how long measure runs each scene")
    parser.add_argument("--console", action="store_true", help="stay attached, printing the app's output")
    args = parser.parse_args()
    if args.command == "check":
        for device in devices():
            print("phone:", device)
        print("identities:", len(identities()))
        for path, info in profiles():
            print("profile:", info.get("Name"), info["Entitlements"]["application-identifier"])
        for folder in PROFILES:
            for path in folder.glob("*provision*") if folder.exists() else []:
                out = run(["security", "cms", "-D", "-i", str(path)], 20)
                if out.returncode == 0:
                    info = plistlib.loads(out.stdout.encode())
                    app_id = info.get("Entitlements", {}).get("application-identifier", "")
                    print("any profile:", info.get("Name"), app_id.split(".", 1)[-1])
        if not profiles():
            print(f"no profile covers {BUNDLE}: see docs/ios-device.md, 'One setup step'")
        return 0
    if args.command == "setup":
        return setup()
    device = pick_device(args.device)
    print("phone:", device["name"], device["model"], "iOS", device["os"], flush=True)
    if args.command == "measure":
        measure(args, device)
        return 0
    app = bundle_app(build(args), args.resource)
    sign(app, device)
    install(app, device)
    environment = dict(pair.split("=", 1) for pair in args.env)
    launch(device, environment, console=args.console)
    print("launched")
    return 0


if __name__ == "__main__":
    sys.exit(main())
