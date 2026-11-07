---
section: added
---
- **`aria_modal = true` marks a sheet or dialog an app builds itself as modal** (`View::accessible_modal` in Rust): while it shows, assistive technology reads it alone, not what lies behind it, as it does `ui.modal` (ARIA's `aria-modal`).
