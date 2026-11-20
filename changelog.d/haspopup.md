---
section: added
---
- **`haspopup` on any node** (zgx, zgx-script, zgx-desktop): `haspopup = "dialog" | "menu" | "listbox" | "tree" | "grid" | true` (`aria-haspopup`; `true` is a menu) says what a control opens, as the accessibility tree's has-popup property (AccessKit's `HasPopup`), in `ui.controls` (`c.haspopup`) and in `zgx inspect node`. A `ui.popover`'s trigger now says "dialog" and a `ui.menu`'s or submenu's "menu" from the moment it mounts (before, only once the lazy panel was built), unless the trigger names its own. `View::accessible_has_popup` for Rust callers; `PopupKind` gains `Listbox`, `Tree` and `Grid`.
