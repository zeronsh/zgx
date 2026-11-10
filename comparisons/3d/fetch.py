#!/usr/bin/env python3
"""Fetch what the 3D comparison draws with, into $ZGX_3D_CACHE
(default ~/.cache/zgx-3d-comparison). Nothing fetched is committed.

- Amazon Lumberyard Bistro v5.2 (NVIDIA ORCA), CC-BY 4.0: "Amazon
  Lumberyard Bistro, Open Research Content Archive (ORCA)", Amazon
  Lumberyard, 2017, http://developer.nvidia.com/orca/amazon-lumberyard-bistro.
  FBX with DDS textures, converted to glTF by convert_bistro.py (needs
  assimp on PATH and Pillow).
- Khronos glTF-Sample-Assets at a pinned commit, sparse: DamagedHelmet,
  CC-BY 4.0. (Not Sponza: its files are under the CRYENGINE Limited License
  Agreement, which grants use for games made with CRYENGINE alone.)
- Poly Haven's "Kloofendal 48d Partly Cloudy (Pure Sky)", 2K Radiance HDR,
  CC0.
- Filament's macOS release (Apache-2.0): its libraries and headers for the
  Filament arm, and cmgen, which prefilters the HDR into the KTX IBL that
  arm loads.

Downloads use a generic client name and are checked against the hashes below.
"""
import hashlib
import os
import subprocess
import sys
import tarfile
import urllib.request
from pathlib import Path

CACHE = Path(os.environ.get("ZGX_3D_CACHE", Path.home() / ".cache" / "zgx-3d-comparison"))
AGENT = "zgx-3d-comparison/0.1"

# The converter the published tables used (run, never linked): Homebrew's
# assimp 6.0.5, which reports itself as "Version 6.0".
ASSIMP = "6.0.5"
KHRONOS = "https://github.com/KhronosGroup/glTF-Sample-Assets.git"
KHRONOS_COMMIT = "8f492390ddc7a2ae221a1369ef560589cf8c70fb"
KHRONOS_MODELS = ["Models/DamagedHelmet/*"]

FILES = {
    "polyhaven/kloofendal_48d_partly_cloudy_puresky_2k.hdr": (
        "https://dl.polyhaven.org/file/ph-assets/HDRIs/hdr/2k/kloofendal_48d_partly_cloudy_puresky_2k.hdr",
        "md5:2eba3a4d7eeb23cbfbeca364c97e7980",
    ),
    "bistro/Bistro_v5_2.zip": (
        # Redirects to a signed download of Bistro_v5_2.zip.
        "https://developer.nvidia.com/bistro",
        "sha256:0d50e3c724c6c5da19f8eb99ad3f53e36fec37ffa2df9621f9ccf0603f3934e1",
    ),
    "filament/filament-v1.80.0-mac.tgz": (
        "https://github.com/google/filament/releases/download/v1.80.0/filament-v1.80.0-mac.tgz",
        "sha256:b514ec6dc8d6026e70f6e0dc9673b1c5748060a4be5137aea8502a46d740fb6b",
    ),
}


def download(path, url, digest):
    target = CACHE / path
    if target.exists():
        return target
    target.parent.mkdir(parents=True, exist_ok=True)
    request = urllib.request.Request(url, headers={"User-Agent": AGENT})
    with urllib.request.urlopen(request) as response:
        data = response.read()
    if digest:
        kind, want = digest.split(":")
        got = hashlib.new(kind, data).hexdigest()
        if got != want:
            raise SystemExit(f"{path}: {kind} {got}, expected {want}")
    target.write_bytes(data)
    print(f"fetched {path} ({len(data) >> 20} MB)")
    return target


def khronos():
    repo = CACHE / "khronos"
    if not repo.exists():
        subprocess.run(["git", "clone", "-q", "--filter=blob:none", "--no-checkout", KHRONOS, repo],
                       check=True)
        subprocess.run(["git", "-C", repo, "sparse-checkout", "set", "--no-cone", *KHRONOS_MODELS],
                       check=True)
    subprocess.run(["git", "-C", repo, "fetch", "-q", "--depth", "1", "origin", KHRONOS_COMMIT],
                   check=True)
    subprocess.run(["git", "-C", repo, "checkout", "-q", KHRONOS_COMMIT], check=True)


def filament():
    archive = download("filament/filament-v1.80.0-mac.tgz", *FILES["filament/filament-v1.80.0-mac.tgz"])
    sdk = CACHE / "filament" / "filament"
    if not sdk.exists():
        with tarfile.open(archive) as tar:
            tar.extractall(CACHE / "filament", filter="data")
    ibl = CACHE / "filament-ibl"
    if not ibl.exists():
        ibl.mkdir()
        hdr = CACHE / "polyhaven/kloofendal_48d_partly_cloudy_puresky_2k.hdr"
        subprocess.run([sdk / "bin" / "cmgen", "--quiet", "--format=ktx", "--size=256",
                        f"--deploy={ibl}", hdr], check=True)


def bistro():
    archive = download("bistro/Bistro_v5_2.zip", *FILES["bistro/Bistro_v5_2.zip"])
    source = CACHE / "bistro" / "Bistro_v5_2"
    if not source.exists():
        import zipfile
        with zipfile.ZipFile(archive) as z:
            z.extractall(CACHE / "bistro")
    if not (CACHE / "bistro/gltf/exterior/exterior.gltf").exists():
        out = subprocess.run(["assimp", "version"], capture_output=True, text=True).stdout
        import re
        found = re.search(r"Version (\d+\.\d+(?:\.\d+)?)", out)
        version = found.group(1) if found else "unknown"
        if not ASSIMP.startswith(version):
            print(f"warning: assimp {version}, the published tables used {ASSIMP}", file=sys.stderr)
        subprocess.run([sys.executable, Path(__file__).with_name("convert_bistro.py"), source,
                        CACHE / "bistro" / "gltf", "--scene", "exterior"], check=True)


def main():
    path = "polyhaven/kloofendal_48d_partly_cloudy_puresky_2k.hdr"
    download(path, *FILES[path])
    khronos()
    bistro()
    if sys.platform == "darwin":
        filament()
    print(f"ready in {CACHE}")


if __name__ == "__main__":
    main()
