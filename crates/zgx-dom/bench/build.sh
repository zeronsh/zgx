#!/bin/sh
# Builds the table benchmark (examples/bench.rs) into bench/out for the
# browser. Needs wasm-bindgen-cli of the version Cargo.lock pins
# (`cargo install wasm-bindgen-cli --version 0.2.127`).
set -eu
here=$(cd "$(dirname "$0")" && pwd)
target=${CARGO_TARGET_DIR:-$here/../../../target}
cargo build --release --locked -p zgx-dom --example bench --target wasm32-unknown-unknown
wasm-bindgen --target web --no-typescript --out-dir "$here/out" \
  "$target/wasm32-unknown-unknown/release/examples/bench.wasm"
ls -l "$here/out"
