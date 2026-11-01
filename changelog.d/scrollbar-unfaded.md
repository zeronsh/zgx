---
section: fixed
---
- **A scroll view's fade leaves its scrollbar whole** (zgx): `fade_edges` on a scroll view or list with a scrollbar (Luau's `fade = { 64, 64 }` with `scrollbar = true`) faded the scrollbar's thumb with the rows, as it passes the fade to all its children. The scrollbar now paints outside its own view's fade; a fade around the view still fades both.
