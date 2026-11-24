#!/bin/sh
# zgx's published crates checked as an app from crates.io builds them: with
# the workspace's `[patch.crates-io]` taken out, so winit, wgpu-hal and
# AccessKit are crates.io's (docs/releasing.md, "What stays patched in the
# repository alone"). A call of a patched crate's own function outside its
# cfg (`zgx_wgpu_hal_patched`, `zgx_winit_patched`) fails here, where the
# repository's own builds and tests have the function:
#
#     scripts/unpatched_check.sh
#
# The tree isn't touched: a copy of the committed one, in the target
# directory, loses the section (and its lockfile the vendored crates, the
# rest as locked).
set -eu
cd "$(git rev-parse --show-toplevel)"
target="${CARGO_TARGET_DIR:-$PWD/target}"
mkdir -p "$target"
copy="$(cd "$target" && pwd)/unpatched"
rm -rf "$copy"
mkdir -p "$copy"
git archive HEAD | tar -x -C "$copy"
cd "$copy"
awk '/^\[/ { skip = ($0 == "[patch.crates-io]") } !skip' Cargo.toml > Cargo.toml.new
mv Cargo.toml.new Cargo.toml
if grep -q '^\[patch' Cargo.toml; then
    echo "a [patch] section is left in the copy's Cargo.toml" >&2
    exit 1
fi
CARGO_TARGET_DIR="$target" cargo check -p zgx-gpu -p zgx-3d -p zgx-terminal \
    -p zgx-script -p zgx-desktop -p zgx-run
for crate in wgpu-hal winit accesskit_consumer; do
    if ! CARGO_TARGET_DIR="$target" cargo metadata --format-version 1 --locked |
        grep -q "\"registry+https://github.com/rust-lang/crates.io-index#$crate@"; then
        echo "$crate isn't crates.io's in the copy" >&2
        exit 1
    fi
done
cd ..
rm -rf "$copy"
