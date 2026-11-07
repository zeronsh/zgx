#!/usr/bin/env python3
"""An App Store app's layout from a zgx app, unsigned: an `.xcarchive` and a
`Payload` .ipa that Xcode's Organizer (or `codesign` and `xcrun altool`) can
sign and upload. Nothing here signs, reads the Keychain or asks for anything.

    python3 scripts/ios/package.py scripts/ios/package/app.example.json
    python3 scripts/ios/package.py app.json --binary path/to/built/app   # no build
    python3 scripts/ios/package.py --check target/ios-package/Synth.xcarchive

The manifest (JSON) names the app and what it needs: `name`, `bundle_id`,
`version`, `build`, `icon` (one square PNG, 1024 points or larger), the cargo
`package` and `example` to build for `aarch64-apple-ios`, `resources` copied
beside the executable (a Luau app's folder), `url_schemes`, and `audio`:
`session_category` (AVAudioSession's, set by the host at launch:
"playback", "playAndRecord", "ambient"…), `background` (keeps playing when
the app goes to the background) and `microphone` (why it records, which iOS
shows when it asks). Paths are the manifest's own folder's.
"""

import argparse
import datetime
import json
import plistlib
import shutil
import string
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
HERE = Path(__file__).resolve().parent / "package"
TARGET = "aarch64-apple-ios"
OUT = ROOT / "target" / "ios-package"
MINIMUM_OS = "26.0"
# AVAudioSession's categories, by the names a manifest gives.
CATEGORIES = {
    "ambient": "AVAudioSessionCategoryAmbient",
    "soloAmbient": "AVAudioSessionCategorySoloAmbient",
    "playback": "AVAudioSessionCategoryPlayback",
    "record": "AVAudioSessionCategoryRecord",
    "playAndRecord": "AVAudioSessionCategoryPlayAndRecord",
    "multiRoute": "AVAudioSessionCategoryMultiRoute",
}


def tool(*args, capture=True):
    result = subprocess.run(args, capture_output=capture, text=True)
    if result.returncode != 0:
        sys.exit(f"{args[0]} {' '.join(args[1:3])} failed:\n{result.stdout}{result.stderr}")
    return result.stdout.strip() if capture else ""


def build(manifest):
    environment = {k: v for k, v in __import__("os").environ.items() if k != "SDKROOT"}
    command = ["cargo", "build", "-p", manifest["package"], "--example", manifest["example"],
               "--target", TARGET, "--locked", "--release"]
    if subprocess.run(command, cwd=ROOT, env=environment).returncode != 0:
        sys.exit("The build failed.")
    return ROOT / "target" / TARGET / "release" / "examples" / manifest["example"]


def sdk_facts():
    xcode = tool("xcodebuild", "-version").splitlines()
    version = xcode[0].split()[-1]
    # DTXcode is the version as four digits: 26.6 → 2660.
    parts = (version.split(".") + ["0", "0"])[:3]
    return {
        "SDK_VERSION": tool("xcrun", "--sdk", "iphoneos", "--show-sdk-version"),
        "SDK_BUILD": tool("xcrun", "--sdk", "iphoneos", "--show-sdk-build-version"),
        "XCODE": f"{int(parts[0]):02d}{int(parts[1])}{int(parts[2])}",
        "XCODE_BUILD": xcode[1].split()[-1],
        "MAC_BUILD": tool("sw_vers", "-buildVersion"),
    }


def info_plist(manifest, executable, icon_keys):
    values = {
        "NAME": manifest["name"],
        "DISPLAY_NAME": manifest.get("display_name", manifest["name"]),
        "EXECUTABLE": executable,
        "BUNDLE_ID": manifest["bundle_id"],
        "VERSION": manifest.get("version", "1.0"),
        "BUILD": str(manifest.get("build", "1")),
        "MINIMUM_OS": MINIMUM_OS,
        **sdk_facts(),
    }
    text = string.Template((HERE / "Info.plist.in").read_text()).substitute(values)
    info = plistlib.loads(text.encode())
    info.update(icon_keys)
    audio = manifest.get("audio", {})
    if audio.get("session_category"):
        category = audio["session_category"]
        if category not in CATEGORIES:
            sys.exit(f"audio.session_category is one of {', '.join(CATEGORIES)}, not {category!r}")
        # The iOS host sets the shared session to it at launch (ios_system.rs).
        info["ZGXAudioSessionCategory"] = CATEGORIES[category]
    if audio.get("background"):
        info["UIBackgroundModes"] = ["audio"]
    if audio.get("microphone"):
        info["NSMicrophoneUsageDescription"] = audio["microphone"]
    if manifest.get("url_schemes"):
        info["CFBundleURLTypes"] = [{
            "CFBundleURLName": manifest["bundle_id"],
            "CFBundleURLSchemes": list(manifest["url_schemes"]),
        }]
    return info


def icons(source, app, work):
    """The app's icon from one image, as an asset catalog compiled into the
    app (`Assets.car`): a 1024 by 1024 PNG with no alpha (the App Store
    refuses one with it), the sizes iOS needs made from it by actool. Returns
    the Info.plist keys actool gives (`CFBundleIcons`…)."""
    width = int(tool("sips", "-g", "pixelWidth", str(source)).split()[-1])
    height = int(tool("sips", "-g", "pixelHeight", str(source)).split()[-1])
    if width != height or width < 1024:
        sys.exit(f"The icon is {width} by {height}: it must be square, 1024 or larger.")
    catalog = work / "Assets.xcassets"
    icon_set = catalog / "AppIcon.appiconset"
    icon_set.mkdir(parents=True)
    (catalog / "Contents.json").write_text(json.dumps({"info": {"author": "xcode", "version": 1}}))
    # Through JPEG and back: the one way sips has of dropping the alpha.
    flat = work / "flat.jpg"
    tool("sips", "-z", "1024", "1024", "-s", "format", "jpeg", "-s", "formatOptions", "best",
         str(source), "--out", str(flat))
    tool("sips", "-s", "format", "png", str(flat), "--out", str(icon_set / "icon-1024.png"))
    (icon_set / "Contents.json").write_text(json.dumps({
        "images": [{"filename": "icon-1024.png", "idiom": "universal", "platform": "ios", "size": "1024x1024"}],
        "info": {"author": "xcode", "version": 1},
    }))
    partial = work / "icons.plist"
    tool("xcrun", "actool", "--compile", str(app), "--platform", "iphoneos",
         "--minimum-deployment-target", MINIMUM_OS, "--app-icon", "AppIcon",
         "--target-device", "iphone", "--target-device", "ipad",
         "--output-partial-info-plist", str(partial), "--output-format", "human-readable-text",
         str(catalog))
    with open(partial, "rb") as f:
        return plistlib.load(f)


def package(manifest_path, binary=None):
    manifest_path = Path(manifest_path).resolve()
    manifest = json.loads(manifest_path.read_text())
    base = manifest_path.parent
    for key in ("name", "bundle_id", "icon"):
        if key not in manifest:
            sys.exit(f"The manifest needs `{key}`.")
    binary = Path(binary) if binary else build(manifest)
    name = manifest["name"]
    executable = "".join(c for c in name if c.isalnum()) or "App"
    archive = OUT / f"{name}.xcarchive"
    shutil.rmtree(archive, ignore_errors=True)
    app = archive / "Products" / "Applications" / f"{executable}.app"
    app.mkdir(parents=True)
    shutil.copy2(binary, app / executable)
    for resource in manifest.get("resources", []):
        source = (base / resource) if (base / resource).exists() else ROOT / resource
        shutil.copytree(source, app / Path(resource).name, symlinks=True)
    with tempfile.TemporaryDirectory() as work:
        icon_keys = icons(base / manifest["icon"], app, Path(work))
    info = info_plist(manifest, executable, icon_keys)
    with open(app / "Info.plist", "wb") as f:
        plistlib.dump(info, f, fmt=plistlib.FMT_BINARY)
    (app / "PkgInfo").write_text("APPL????")
    # Symbols for crash reports, beside the app as Xcode keeps them.
    dsyms = archive / "dSYMs"
    dsyms.mkdir()
    tool("xcrun", "dsymutil", str(binary), "-o", str(dsyms / f"{executable}.app.dSYM"))
    with open(archive / "Info.plist", "wb") as f:
        plistlib.dump({
            "ApplicationProperties": {
                "ApplicationPath": f"Applications/{executable}.app",
                "Architectures": ["arm64"],
                "CFBundleIdentifier": manifest["bundle_id"],
                "CFBundleShortVersionString": info["CFBundleShortVersionString"],
                "CFBundleVersion": info["CFBundleVersion"],
                "SigningIdentity": "",
            },
            "ArchiveVersion": 2,
            "CreationDate": datetime.datetime.now(datetime.timezone.utc).replace(tzinfo=None),
            "Name": name,
            "SchemeName": name,
        }, f)
    shutil.copy2(HERE / "entitlements.plist", OUT / f"{name}.entitlements")
    # The same app as an unsigned .ipa: Payload/<app>.app, zipped.
    ipa = OUT / f"{name}-unsigned.ipa"
    with tempfile.TemporaryDirectory() as work:
        payload = Path(work) / "Payload"
        shutil.copytree(app, payload / app.name, symlinks=True)
        ipa.unlink(missing_ok=True)
        tool("ditto", "-c", "-k", "--sequesterRsrc", "--keepParent", str(payload), str(ipa))
    check(archive)
    print(f"{archive}\n{ipa}\n{OUT / (name + '.entitlements')}\nunsigned: sign and upload with Xcode's Organizer")


def check(archive):
    """The archive's structure, as an upload would read it: every plist
    well formed, the app where the archive says, its keys, an arm64 iOS
    executable, the compiled icons and the symbols."""
    archive = Path(archive)
    problems = []
    plists = [p for p in archive.rglob("*.plist")] + [archive / "Info.plist"]
    for plist in dict.fromkeys(plists):
        lint = subprocess.run(["plutil", "-lint", str(plist)], capture_output=True, text=True)
        if lint.returncode != 0:
            problems.append(lint.stdout.strip())
    with open(archive / "Info.plist", "rb") as f:
        meta = plistlib.load(f)
    app = archive / "Products" / meta["ApplicationProperties"]["ApplicationPath"]
    if not app.is_dir():
        sys.exit(f"No app at {app}")
    with open(app / "Info.plist", "rb") as f:
        info = plistlib.load(f)
    for key in ("CFBundleIdentifier", "CFBundleExecutable", "CFBundleShortVersionString", "CFBundleVersion",
                "MinimumOSVersion", "CFBundleIcons", "UILaunchScreen", "DTPlatformName"):
        if key not in info:
            problems.append(f"Info.plist lacks {key}")
    if info.get("CFBundleIdentifier") != meta["ApplicationProperties"]["CFBundleIdentifier"]:
        problems.append("the archive and the app name different bundle ids")
    executable = app / info.get("CFBundleExecutable", "?")
    archs = subprocess.run(["lipo", "-archs", str(executable)], capture_output=True, text=True).stdout.split()
    if archs != ["arm64"]:
        problems.append(f"the executable is {archs or 'missing'}, not arm64")
    build_version = subprocess.run(["xcrun", "vtool", "-show-build", str(executable)],
                                   capture_output=True, text=True).stdout
    if "platform IOS" not in build_version or "IOSSIMULATOR" in build_version:
        problems.append("the executable isn't built for iOS devices")
    if not (app / "Assets.car").is_file():
        problems.append("no compiled asset catalog (Assets.car)")
    if not any((archive / "dSYMs").glob("*.dSYM")):
        problems.append("no dSYM")
    if (app / "_CodeSignature").exists():
        problems.append("the app is signed: this layout is meant unsigned")
    for line in problems:
        print("problem:", line)
    if problems:
        sys.exit(1)
    print(f"ok: {archive.name}: {info['CFBundleIdentifier']} {info['CFBundleShortVersionString']} "
          f"({info['CFBundleVersion']}), arm64 iOS, icons, dSYM, plists lint clean, unsigned")


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("manifest", nargs="?")
    parser.add_argument("--binary", help="an already built aarch64-apple-ios executable: skips the build")
    parser.add_argument("--check", metavar="XCARCHIVE", help="check an archive's structure, nothing built")
    args = parser.parse_args()
    if args.check:
        check(args.check)
    elif args.manifest:
        package(args.manifest, args.binary)
    else:
        parser.error("a manifest, or --check")


if __name__ == "__main__":
    main()
