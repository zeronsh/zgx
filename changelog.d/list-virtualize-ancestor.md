---
section: added
---
- **A list can scroll with the scroll view around it** (zgx, zgx-script): `ui.list { …, virtualize = "ancestor" }` (Rust: `View::virtualize_in_ancestor`) is as tall as all its rows and builds only those the scroll view around it shows, plus its overscan, again as that view scrolls or resizes. A panel that scrolls as a whole, its sections lists, builds about 20 rows of 302 instead of all of them. With `height` or `heights`: Tab and focus go through its rows, a row asked for (`scroll_to_row`) or found in the page scrolls that view to it, and a reload finds focus again on its row. Opt-in: a list's rows otherwise build as before.
