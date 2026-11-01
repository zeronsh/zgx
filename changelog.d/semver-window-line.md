---
section: changed
---
- **`zgx_script::Window` and `driver::Line` keep 0.1.6's fields** (zgx-script): code building them with a struct literal compiles as before. A window's `under_pointer`, `visible` and own `state` are read with `ScriptHost::window_extras(id)` (a `WindowExtras`, `non_exhaustive` with `Default`, beside `window_size_limits`), and whether a tree line's node shows with `driver::tree_lines`, whose `TreeLine` holds the `Line` and its `hidden` flag; `tree` and `tree_text` are as in 0.1.6.
