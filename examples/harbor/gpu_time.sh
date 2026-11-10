#!/bin/sh
# harbor_bench with GPU busy time (macOS): built against zgx's vendored
# wgpu-hal with the perf lane's instrument applied
# (comparisons/3d/wgpu-hal-gpu-time.patch), as the 3D comparison's arms
# are. The copy is made under the target directory; nothing tracked
# changes (the lockfile is put back as it was).
#
#   ./gpu_time.sh            # then run with the bench's usual settings:
#   FRAMES=600 ./gpu_time.sh
set -eu
here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../.." && pwd)
target=${CARGO_TARGET_DIR:-"$here/target"}
timed="$target/wgpu-hal-timed"
patch="$root/comparisons/3d/wgpu-hal-gpu-time.patch"
if ! cmp -s "$patch" "$timed/.patched" 2>/dev/null; then
    rm -rf "$timed"
    mkdir -p "$target"
    cp -R "$root/vendor/wgpu-hal" "$timed"
    patch -s -p1 -d "$timed" -i "$patch"
    cp "$patch" "$timed/.patched"
fi
cp "$here/Cargo.lock" "$target/harbor-Cargo.lock.saved"
trap 'cp "$target/harbor-Cargo.lock.saved" "$here/Cargo.lock"' EXIT
cd "$here"
CARGO_TARGET_DIR="$target/gpu-time" cargo build --release --offline \
    --features gpu-time --bin harbor_bench \
    --config "patch.crates-io.wgpu-hal.path=\"$timed\""
"$target/gpu-time/release/harbor_bench"
