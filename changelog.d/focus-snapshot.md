---
section: added
---
- **Focus and a field's selection can be put back as they were** (zgx, zgx-script): `Ui::focus_snapshot` / `Ui::restore_focus` (hidden, internal) and Luau `ui.focus_snapshot()` with `:restore()` (internal, subject to change) take what has keyboard focus and a text field's, textarea's or code editor's selection, and put them back later without scrolling; false if the node is gone or its text no longer holds the offsets.
