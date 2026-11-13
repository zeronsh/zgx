# zgx patch to luau0-src 0.22.0+luau740

Vendored from crates.io `luau0-src` (MIT, license file unchanged), published
as `zgx-luau0-src` (its library still `luau0_src`), which `zgx-mlua-sys` and
`zgx-mlua` depend on.

## What

Release builds (build scripts without debug assertions) define `NDEBUG`
for Luau. Luau checks its asserts unless `NDEBUG` is defined, and `cc` does
not define it: with them every script loop, call and table access runs about
a fifth slower (`list_state_bench`: 6.3 M instructions a tick against
5.0 M). Debug builds keep them (`LUAU_ENABLE_ASSERT`), as upstream. A build
that wants the asserts in release adds `-UNDEBUG` to `LUAU_CXXFLAGS`.
