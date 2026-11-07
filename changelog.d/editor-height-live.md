---
section: fixed
---
- **A one-line field follows `control.height` and the system text size as they change** (zgx): its box kept the height it was built at, so a larger text size clipped its line. `control.height` now also grows, under a system text size above 1, to hold a body line and its padding, as iOS's controls grow with Dynamic Type.
