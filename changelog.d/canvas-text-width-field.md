---
section: changed
---
- **`CanvasText` has a `width`** (zgx): a Rust canvas making its text with a struct literal adds `width: f32::INFINITY` for a line as before (a bare `f32`, so a line of text is no bigger than before).
