#!/bin/sh
# Builds the table benchmark (examples/bench.rs) into bench/out for the
# browser, or with `luau` the Luau app (examples/luau.rs) into bench/luau/out
# (with luau-env.sh sourced), or with `luau-bench` the table in Luau
# (examples/luau_bench.rs) into bench/luau-table/out. Needs wasm-bindgen-cli of the version
# Cargo.lock pins (`cargo install wasm-bindgen-cli --version 0.2.127`).
set -eu
here=$(cd "$(dirname "$0")" && pwd)
target=${CARGO_TARGET_DIR:-$here/../../../target}
if [ "${1:-}" = luau ]; then
  example=luau out="$here/luau/out" features="--features luau"
elif [ "${1:-}" = luau-bench ]; then
  example=luau_bench out="$here/luau-table/out" features="--features luau"
else
  example=bench out="$here/out" features=""
fi
# shellcheck disable=SC2086
cargo build --release --locked -p zgx-dom --example "$example" $features --target wasm32-unknown-unknown
wasm-bindgen --target web --no-typescript --out-dir "$out" \
  "$target/wasm32-unknown-unknown/release/examples/$example.wasm"
ls -l "$out"
