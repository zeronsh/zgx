#!/usr/bin/env python3
"""Prepares a build of Ladybird's unmodified source for zgx, and checks what it installed.

  prepare.py manifest <ladybird source> <out dir>
      Writes <out>/manifest (Ladybird's vcpkg manifest without what zgx leaves out, see LEFT_OUT) and
      <out>/overlay-ports, for -DVCPKG_MANIFEST_DIR=<out>/manifest.
  prepare.py hsts <cache dir>
      Puts the HSTS preload list, at a pinned Chromium commit and checked against its hash, where Ladybird's build
      reads it (<cache>/HSTSPreload), so that it never takes the list from Chromium's main branch.
  prepare.py check <vcpkg installed dir> <triplet>
      Fails if a left-out port was installed, or if FFmpeg was built under anything but the LGPL 2.1.
"""

import hashlib
import json
import os
import shutil
import sys
import urllib.request
from pathlib import Path

# Ports zgx's build leaves out of Ladybird's, and why. Each would bring a copyleft licence into an app's engine.
LEFT_OUT = {
    "cpptrace": "only for stack traces; brings libdwarf (LGPL-2.1). Ladybird falls back to backtrace.h",
    "libdwarf": "only for cpptrace (LGPL-2.1)",
    "qtbase": "only for Ladybird's Qt browser (LGPL-3)",
    "qtpositioning": "only for Ladybird's Qt browser (LGPL-3)",
    "sparkle": "only for updating Ladybird's own macOS app",
}
# Ports that must not be installed, whatever brings them.
FORBIDDEN = ["libdwarf", "libidn2", "libunistring", "gettext-libintl", "qtbase", "qtpositioning", "qtsvg", "qtserialport"]

HSTS_COMMIT = "ba9567c435afe2bb2e71d1a348b3c65790a572db"  # Chromium's last change to the list before Ladybird 2026.47.8
HSTS_URL = f"https://raw.githubusercontent.com/chromium/chromium/{HSTS_COMMIT}/net/http/transport_security_state_static.json"
HSTS_SHA256 = "4ca12234c89e6367bcdcf1b899326cb46765a7441188c1524c9bdfed39fb3f31"


def dependency_name(dependency):
    return dependency if isinstance(dependency, str) else dependency["name"]


def manifest(source: Path, out: Path):
    lb_manifest = json.loads((source / "vcpkg.json").read_text())
    dependencies = []
    for dependency in lb_manifest["dependencies"]:
        name = dependency_name(dependency)
        if name in LEFT_OUT:
            continue
        if name == "libpsl":
            # libpsl's default runtime on Linux and macOS is libidn2 (LGPL-3, with libunistring). ICU, which Ladybird
            # ships anyway, does the same work.
            dependency = {"name": "libpsl", "default-features": False, "features": ["libicu"]}
        dependencies.append(dependency)
    lb_manifest["dependencies"] = dependencies
    lb_manifest.pop("features", None)  # qt and sparkle, both left out
    lb_manifest.pop("default-features", None)

    manifest_dir = out / "manifest"
    overlays = out / "overlay-ports"
    if overlays.exists():
        shutil.rmtree(overlays)
    manifest_dir.mkdir(parents=True, exist_ok=True)

    # Ladybird's libpsl port allows ICU on Windows alone; the port's build takes it anywhere.
    libpsl = overlays / "libpsl"
    shutil.copytree(source / "Meta/CMake/vcpkg/overlay-ports/libpsl", libpsl)
    port = json.loads((libpsl / "vcpkg.json").read_text())
    port["features"]["libicu"].pop("supports", None)
    port.pop("default-features", None)
    (libpsl / "vcpkg.json").write_text(json.dumps(port, indent=2) + "\n")

    configuration = json.loads((source / "vcpkg-configuration.json").read_text())
    configuration["overlay-ports"] = [str(overlays), str(source / "Meta/CMake/vcpkg/overlay-ports")]
    (manifest_dir / "vcpkg.json").write_text(json.dumps(lb_manifest, indent=2) + "\n")
    (manifest_dir / "vcpkg-configuration.json").write_text(json.dumps(configuration, indent=2) + "\n")
    print(f"manifest: {manifest_dir} (left out: {', '.join(sorted(LEFT_OUT))}; libpsl on ICU)", file=sys.stderr)


def sha256(path: Path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def hsts(cache: Path):
    target = cache / "HSTSPreload" / "transport_security_state_static.json"
    if target.exists() and sha256(target) == HSTS_SHA256:
        return
    target.parent.mkdir(parents=True, exist_ok=True)
    temporary = target.with_suffix(".tmp")
    with urllib.request.urlopen(HSTS_URL) as response:
        temporary.write_bytes(response.read())
    digest = sha256(temporary)
    if digest != HSTS_SHA256:
        temporary.unlink()
        sys.exit(f"HSTS preload list at {HSTS_COMMIT}: sha256 {digest}, expected {HSTS_SHA256}")
    temporary.replace(target)
    print(f"hsts: {target} (Chromium {HSTS_COMMIT[:12]})", file=sys.stderr)


def check_plan(plan: str):
    """Fails if vcpkg's install plan (`vcpkg install --dry-run`'s output) has a left-out port."""
    planned = []
    for line in plan.splitlines():
        words = line.strip().lstrip("*").strip().split(":", 1)
        if len(words) == 2 and words[0]:
            planned.append(words[0].split("[", 1)[0])
    found = sorted(set(planned) & (set(FORBIDDEN) | set(LEFT_OUT)))
    if found:
        sys.exit(f"left-out ports are in vcpkg's plan: {', '.join(found)}")


def check(installed: Path, triplet: str):
    status = (installed / "vcpkg" / "status").read_text()
    ports = set()
    for paragraph in status.split("\n\n"):
        fields = dict(line.split(": ", 1) for line in paragraph.splitlines() if ": " in line)
        if fields.get("Architecture") == triplet and fields.get("Status", "").endswith(" installed"):
            ports.add(fields["Package"])
    found = sorted(set(FORBIDDEN) & ports)
    if found:
        sys.exit(f"left-out ports were installed for {triplet}: {', '.join(found)}")
    ffmpeg_copyright = installed / triplet / "share" / "ffmpeg" / "copyright"
    if "ffmpeg" in ports:
        # FFmpeg's configure says the licence it built under; vcpkg keeps its log when it built FFmpeg here (not when
        # its binary cache had it, whose copyright file the portfile picked from the same line).
        logs = list(Path(installed).parent.glob("vcpkg*/buildtrees/ffmpeg/build-*-rel-out.log"))
        logs += list(Path(os.environ.get("VCPKG_ROOT", "/nonexistent")).glob("buildtrees/ffmpeg/build-*-rel-out.log"))
        for log in logs:
            lines = [line for line in log.read_text(errors="replace").splitlines() if line.startswith("License:")]
            if lines and lines[0].strip() != "License: LGPL version 2.1 or later":
                sys.exit(f"FFmpeg's configure says {lines[0]!r}, not the LGPL 2.1: see {log}")
        text = ffmpeg_copyright.read_text()
        if "GNU LESSER GENERAL PUBLIC LICENSE" not in text or "Version 2.1" not in text:
            sys.exit(f"FFmpeg was built under another licence than the LGPL 2.1: see {ffmpeg_copyright}")
    print(f"check: {len(ports)} ports for {triplet}, none of {', '.join(FORBIDDEN)}; FFmpeg LGPL-2.1", file=sys.stderr)


if __name__ == "__main__":
    command, *arguments = sys.argv[1:]
    if command == "manifest":
        manifest(Path(arguments[0]).resolve(), Path(arguments[1]).resolve())
    elif command == "hsts":
        hsts(Path(arguments[0]).resolve())
    elif command == "check":
        check(Path(arguments[0]).resolve(), arguments[1])
    else:
        sys.exit(__doc__)
