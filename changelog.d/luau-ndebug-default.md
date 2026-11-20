---
section: fixed
---
- **Release builds run Luau without its asserts by default** (zgx-script): Luau checks its asserts unless `NDEBUG` is defined and `cc` never defined it, so scripts ran about a fifth slower in any release build that didn't set `LUAU_CXXFLAGS=-DNDEBUG` (an install, an app depending on zgx), and `zgx test ui` said so. zgx now builds Luau from its own copies of `luau0-src` and `mlua-sys` (`zgx-luau0-src`, `zgx-mlua-sys`), which define `NDEBUG` in release builds; debug builds keep the asserts. The warning and `build.rs` that detected the missing flag are gone, and the docs no longer ask for it. (`zgx-luau0-src` and `zgx-mlua-sys` must be published before `zgx-mlua`.)
