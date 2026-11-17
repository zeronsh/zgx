---
section: added
---
- **The frame log names an effect no maker named by the Rust line that made it** (`zgx inspect frames`, `zgx::trace`): `zgx/src/compose.rs:812 effect` where it said `effect 124`, as #2257 did for writes. A binding's or a script's own name still wins. Only in `trace` builds (devtools, `inspect`): `Runtime::effect` and `effect_now` take their caller there alone.
