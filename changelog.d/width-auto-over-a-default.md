---
section: fixed
---
- **`w = "auto"` and `"max-content"` replace an overlay panel's default width** (zgx, zgx-script): an anchored menu or popover panel is 240 wide by default, and a style that set its width to `auto` (a script's `w = "auto"`, in a node's props or a style table such as `context_menu_style`) was ignored, as was a calculated width (`max-content`, `fit-content(320px)`): the panel kept 240. `Styled::w_auto`/`h_auto` are new, and merging styles now lets a set width override one under it even when it is `auto`. A popover now sizes to what it holds. A menu's panel (and a context menu's) still does not shrink to its rows, only to its padding: its viewport reports no width of its own, a layout problem left for its own change.
