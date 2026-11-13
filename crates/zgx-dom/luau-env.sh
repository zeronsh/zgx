# Source this, with WASI_SDK naming a wasi-sdk (34 or later), to build
# zgx-script's Luau VM for the browser (wasm32-unknown-unknown): its C++ is
# compiled by wasi-sdk's clang against the wasi sysroot's libc and libc++,
# with WebAssembly exceptions (Luau throws on script errors), and linked
# with them. Only wasm32 builds read these; native builds are as they were.
#
#   WASI_SDK=/opt/wasi-sdk . crates/zgx-dom/luau-env.sh
#   cargo test -p zgx-dom --features luau --target wasm32-unknown-unknown
: "${WASI_SDK:?set WASI_SDK to a wasi-sdk directory}"
zgx_sysroot="$WASI_SDK/share/wasi-sysroot"
export CC_wasm32_unknown_unknown="$WASI_SDK/bin/clang"
export CXX_wasm32_unknown_unknown="$WASI_SDK/bin/clang++"
export AR_wasm32_unknown_unknown="$WASI_SDK/bin/llvm-ar"
export CFLAGS_wasm32_unknown_unknown="--target=wasm32-wasip1 --sysroot=$zgx_sysroot"
export CXXFLAGS_wasm32_unknown_unknown="--target=wasm32-wasip1 --sysroot=$zgx_sysroot -fexceptions -fwasm-exceptions -mllvm -wasm-use-legacy-eh=false"
export CXXSTDLIB_wasm32_unknown_unknown=c++
export CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUSTFLAGS="-L $zgx_sysroot/lib/wasm32-wasip1/eh -L $zgx_sysroot/lib/wasm32-wasip1 -l static=c++abi -l static=unwind -l static=c"
unset zgx_sysroot
