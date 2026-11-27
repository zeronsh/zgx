# zgx patch to mlua-sys 0.13.0

Vendored from crates.io `mlua-sys` 0.13.0 (MIT, `LICENSE` as `vendor/mlua`'s, the
same project) and published as `zgx-mlua-sys` 0.13.1, its library still
`mlua_sys` and its `links` still `lua`. `zgx-mlua` depends on it. The change is
marked `zgx patch` in the source.

## Luau without its asserts in a release build

Luau's `LUAU_ASSERT` is live unless `NDEBUG` is defined (`Common.h`), and
luau0-src, which mlua-sys's build script calls to compile Luau, defines it only
from `LUAU_CXXFLAGS`. The repository set that in `.cargo/config.toml`, which no
published crate carries: every app built from crates.io, and every `cargo
install zgx-run`, ran scripts with the asserts, about a fifth slower
(`list_state_bench`, 4.22 M instructions a tick against 3.07 M, 0.18 ms against 0.15 ms on an Apple M5 Max).

`build/find_vendored.rs` defines it before calling luau0-src, in the same
process, unless the crate is built with debug assertions
(`CARGO_CFG_DEBUG_ASSERTIONS`); a `LUAU_CXXFLAGS` the caller sets is kept beside
it. It prints `ndebug` as `links` metadata, which zgx-script reads
(`DEP_LUA_NDEBUG`) to warn of a release build that has the asserts anyway.
