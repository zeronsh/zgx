---
section: added
---
- **`label` on a rich-text span** (zgx, zgx-script): `ui.span { label = "Citation 1, Sleep study.pdf", on_click = … }` is what assistive technology reads for that run in place of its text, as `aria-label` is. `TextSpan::label` in Rust. A static string for now.
