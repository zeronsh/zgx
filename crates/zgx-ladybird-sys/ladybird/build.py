#!/usr/bin/env python3
"""Builds zgx's Ladybird host with Ladybird at its pinned release, unmodified, and installs them.

  build.py [--print-install-dir] [--jobs N]

Everything goes under the cache (ZGX_LADYBIRD_CACHE, else ~/.cache/zgx-ladybird or ~/Library/Caches/zgx-ladybird):
Ladybird's source at its commit, vcpkg at the baseline Ladybird's manifest names, the build, and the install, which a
later build with the same release and host source reuses. Nothing of Ladybird's is kept in zgx's repository.

The build leaves out of Ladybird what would bring a copyleft licence (prepare.py's LEFT_OUT), builds zgx's host in
place of Ladybird's Qt browser (zgx-project-include.cmake), and fails if a left-out port is planned or installed, if
FFmpeg isn't LGPL-2.1, or if Ladybird's list of third-party notices (LICENSES.txt) wasn't made.

Needs: git, CMake 3.30+, Ninja, a C++23 compiler (clang 20+ or GCC 16; Xcode 26 on macOS), Rust, Python 3, and what
vcpkg's ports build with (nasm, autoconf, automake, libtool, pkg-config).
"""

import argparse
import hashlib
import os
import platform
import shutil
import subprocess
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
CRATE = HERE.parent
sys.path.insert(0, str(HERE))
import prepare  # noqa: E402

RELEASE = "2026.47.8"
COMMIT = "0be55a57cc07ed258362c631a13a10bb4f6d707a"
REPOSITORY = "https://github.com/LadybirdBrowser/ladybird.git"
VCPKG_REPOSITORY = "https://github.com/microsoft/vcpkg.git"


def log(message):
    print(f"zgx-ladybird-sys: {message}", file=sys.stderr, flush=True)


def run(command, cwd=None, env=None):
    log("$ " + " ".join(str(c) for c in command))
    subprocess.run([str(c) for c in command], cwd=cwd, env=env, check=True, stdout=sys.stderr)


def cache_dir():
    if os.environ.get("ZGX_LADYBIRD_CACHE"):
        return Path(os.environ["ZGX_LADYBIRD_CACHE"])
    if platform.system() == "Darwin":
        return Path.home() / "Library/Caches/zgx-ladybird"
    return Path(os.environ.get("XDG_CACHE_HOME", Path.home() / ".cache")) / "zgx-ladybird"


def host_hash():
    """What a change to zgx's host or build glue changes: the install's name."""
    digest = hashlib.sha256()
    for path in sorted(list((CRATE / "host").rglob("*")) + list(HERE.rglob("*"))):
        if path.is_file() and "__pycache__" not in path.parts:
            digest.update(str(path.relative_to(CRATE)).encode())
            digest.update(path.read_bytes())
    return digest.hexdigest()[:12]


def checkout(repository, commit, into: Path):
    if (into / ".git").exists():
        head = subprocess.run(["git", "rev-parse", "HEAD"], cwd=into, capture_output=True, text=True).stdout.strip()
        if head == commit:
            return
    else:
        into.mkdir(parents=True, exist_ok=True)
        run(["git", "init", "-q"], cwd=into)
        run(["git", "remote", "add", "origin", repository], cwd=into)
    run(["git", "fetch", "-q", "--depth", "1", "origin", commit], cwd=into)
    run(["git", "checkout", "-q", "--force", "FETCH_HEAD"], cwd=into)


def checkout_vcpkg(commit, into: Path):
    """vcpkg reads the ports' older versions out of its history, so it is a whole clone, unlike Ladybird's."""
    if not (into / ".git").exists():
        run(["git", "clone", "-q", VCPKG_REPOSITORY, into])
    elif subprocess.run(["git", "cat-file", "-e", f"{commit}^{{commit}}"], cwd=into).returncode != 0:
        run(["git", "fetch", "-q", "origin"], cwd=into)
    run(["git", "checkout", "-q", "--force", commit], cwd=into)


def triplet():
    machine = platform.machine().lower()
    arch = "arm64" if machine in ("arm64", "aarch64") else "x64"
    system = "osx" if platform.system() == "Darwin" else "linux"
    return f"{arch}-{system}-dynamic"


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--print-install-dir", action="store_true")
    parser.add_argument("--jobs", type=int, default=int(os.environ.get("ZGX_LADYBIRD_JOBS", os.cpu_count() or 4)))
    arguments = parser.parse_args()

    cache = cache_dir()
    install = cache / f"install-{RELEASE}-{host_hash()}"
    if (install / ".complete").exists():
        if arguments.print_install_dir:
            print(install)
        return

    source = cache / f"ladybird-{RELEASE}"
    checkout(REPOSITORY, COMMIT, source)
    baseline = __import__("json").loads((source / "vcpkg.json").read_text())["builtin-baseline"]
    vcpkg = cache / "vcpkg"
    checkout_vcpkg(baseline, vcpkg)
    vcpkg_tool = vcpkg / ("vcpkg.exe" if os.name == "nt" else "vcpkg")
    if not vcpkg_tool.exists():
        run([vcpkg / "bootstrap-vcpkg.sh", "-disableMetrics"], cwd=vcpkg)

    work = cache / f"work-{RELEASE}"
    build = work / "build"
    downloads = cache / "downloads"
    prepare.manifest(source, work)
    prepare.hsts(downloads)

    env = dict(os.environ)
    env["VCPKG_ROOT"] = str(vcpkg)
    env["LADYBIRD_SOURCE_DIR"] = str(source)
    env["CMAKE_BUILD_PARALLEL_LEVEL"] = str(arguments.jobs)
    env["VCPKG_MAX_CONCURRENCY"] = str(arguments.jobs)
    # Cargo's variables for this build script would steer Ladybird's own Rust builds.
    for key in list(env):
        if key.startswith("CARGO_") and key not in ("CARGO_HOME",) or key in ("RUSTFLAGS", "RUSTC_WRAPPER", "TARGET", "HOST", "OUT_DIR"):
            env.pop(key)

    # Refuse before building anything if a left-out port is still planned.
    plan = subprocess.run(
        [vcpkg_tool, "install", "--dry-run", f"--x-manifest-root={work / 'manifest'}",
         f"--overlay-triplets={source / 'Meta/CMake/vcpkg/release-triplets'}",
         f"--triplet={triplet()}", f"--host-triplet={triplet()}", f"--x-install-root={work / 'plan-root'}"],
        env=env, capture_output=True, text=True, check=True).stdout
    prepare.check_plan(plan)

    run(["cmake", "-S", source, "-B", build, "-G", "Ninja",
         "-DCMAKE_BUILD_TYPE=Release",
         "-DLADYBIRD_VCPKG_TYPE=release",
         f"-DVCPKG_MANIFEST_DIR={work / 'manifest'}",
         f"-DLADYBIRD_CACHE_DIR={downloads}",
         f"-DCMAKE_PROJECT_ladybird_INCLUDE={HERE / 'zgx-project-include.cmake'}",
         f"-DZGX_LADYBIRD_HOST_DIR={CRATE / 'host'}",
         f"-DQt6_DIR={HERE / 'qt-stub'}",
         "-DLADYBIRD_ENABLE_CPPTRACE=OFF",
         "-DLADYBIRD_ENABLE_UPDATE_CHECK=OFF",
         "-DLADYBIRD_ENABLE_SPARKLE=OFF",
         "-DBUILD_TESTING=OFF",
         "-DENABLE_INSTALL_FREEDESKTOP_FILES=OFF",
         f"-DCMAKE_INSTALL_PREFIX={install}"], env=env)
    prepare.check(build / "vcpkg_installed", triplet())
    run(["ninja", "-C", build, "-j", arguments.jobs, "ladybird", "generate_licenses"], env=env)

    if install.exists():
        shutil.rmtree(install)
    run(["cmake", "--install", build, "--component", "zgx_ladybird"], env=env)
    licenses = install / "LICENSES.txt"
    if not licenses.exists() or licenses.stat().st_size == 0:
        sys.exit(f"zgx-ladybird-sys: Ladybird's third-party notices weren't installed at {licenses}")
    (install / ".complete").write_text(f"{RELEASE} {COMMIT}\n")
    log(f"installed in {install}")
    if arguments.print_install_dir:
        print(install)


if __name__ == "__main__":
    main()
