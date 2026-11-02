---
section: fixed
---
- **A reload keeps focus where it was** (zgx, zgx-script): reloading a script built the interface again, and focus fell to its first Tab stop, so a focused `ui.list` row, or any control but the first, lost the keyboard on every save. What was focused is now found again once the new interface settles: in a list's row, the same row by its index (scrolled to if the list hasn't built it) and the same place in it, so one of many alike buttons (a "Delete" per row) keeps the right one; elsewhere the one node of its role and accessible name if it is the only one, else the node at its place. Rust views get it with `compose::switch_keeping_focus`.
