---
section: added
---
- **A text area's atom is read as one unit, by its name where it has one** (zgx, zgx-desktop, zgx-dom, zgx-script): to a screen reader each chip is one character of its field's text (AccessKit) or one named image in the browser's (`role="img"`, `aria-label`), so it is stepped over and read whole, not as "at, f, i, l, e…". An atom's new `name` (in `atoms` items and `ui.editor.insert_atom`'s options) is what is read for it instead of its text: a mention's full name. It is kept through undo and the clipboard.
