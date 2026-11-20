---
section: fixed
---
- **A menu sized by what it holds is as wide as its widest row** (zgx): a menu's scrolling viewport held its content at the panel's width, so a panel at `w = "max-content"` (a context menu sized to its labels) stayed at its `min_w`. The content is now at least the panel's width, and otherwise its own.
