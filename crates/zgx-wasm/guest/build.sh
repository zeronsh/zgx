#!/bin/sh
# Builds the guests (rustup's toolchain with `wasm32-wasip2`:
# `rustup target add wasm32-wasip2`) into built/.
set -eu
cd "$(dirname "$0")"
PATH="$HOME/.cargo/bin:$PATH" cargo build --release --target wasm32-wasip2 "$@"
mkdir -p built
for app in cube pixels misbehave features; do
    cp "${CARGO_TARGET_DIR:-target}/wasm32-wasip2/release/$app.wasm" "built/$app.wasm"
done
ls -l built
