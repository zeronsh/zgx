---
section: fixed
---
- **Tab and Shift+Tab go on through a list's rows, built or not** (zgx): Tab only knew the rows a `ui.list` had built, so past them it left the list (or wrapped round to the composer) halfway down a transcript. Now, at the last built row that way, the next row is scrolled to, built, and its first control (its last, with Shift+Tab) focused with the same key, rows with nothing to focus passed over, as a page's Tab goes on down a list it holds whole. A list whose rows are one Tab stop (`keyboard = true`) is left by Tab as before.
