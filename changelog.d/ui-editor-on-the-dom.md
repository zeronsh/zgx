---
section: fixed
---
- **`ui.editor.insert_atom`, `atoms`, `undo`, `redo`, `can_undo` and `can_redo` work on the DOM** (zgx-dom, zgx-script): a textarea with `atoms` in the browser takes a script's chip (replacing the typed `@que`, one undo step) and its undo, as on the desktop; they returned nil or false there. A plain browser textarea (no `atoms`) keeps the browser's own undo, which a script can't reach.
