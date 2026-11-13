---
section: added
---
- **A menu's scroll extent for scripts** (zgx, zgx-script): `ui.window.popups` entries and `ui.controls()` entries of role menu or listbox carry `offset`/`range` (`scroll_offset`/`scroll_range` on a control) and `at_start`/`at_end` when they scroll, so a script can tell whether a menu cut to a height can still scroll up or down. A menu's own scroller registers with `Ui::scroll_report`.
