---
section: fixed
---
- **Release builds run Luau without its asserts, with nothing to set** (zgx-mlua, the new zgx-mlua-sys): an app built from crates.io, and `cargo install zgx-run` or `zgx`, ran scripts with Luau's asserts unless `LUAU_CXXFLAGS=-DNDEBUG` was set, about a fifth slower (`list_state_bench`: 4.22 M instructions a tick against 3.07 M, 0.18 ms against 0.15 ms on an Apple M5 Max), as only the repository's own `.cargo/config.toml` set it. zgx-mlua-sys, mlua-sys with one change, now defines `NDEBUG` for Luau wherever the build has no debug assertions; debug builds keep them. The install commands lose the variable.
