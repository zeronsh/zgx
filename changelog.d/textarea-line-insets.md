---
section: added
---
- **A textarea's lines can take room of their own from Luau** (zgx-script): `line_insets = function(line, state) return { left, above, below, hidden } end` gives a line an indent, space above and below, or hides it, from its text and the state `highlight` was given for it (a fenced block's lines know they're inside it). It's asked once for each line text and state; a textarea without it lays out as before.
