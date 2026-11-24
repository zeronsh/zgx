---
section: added
---
- **`zgx inspect frames` names the rest of a frame's writes by the Rust line that made their signals** (`31 × zgx-script/src/lower.rs:8290`), where no script line or kind names them: an engine's per-row or per-block state reads as where it comes from, not "31 unnamed". Devtools only (`trace`): default builds are unchanged.
