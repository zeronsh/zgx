---
section: added
---
- **A region's `on_copy` hears the list rows a selection spans** (zgx-script): `request.rows` holds, for each `ui.list` in the region the selection covers, `{ list, first, last }`: the list's id and its first and last rows, from 1, whether mounted or scrolled away. A Select All over a list of thousands names every row, so an app finds the citations of rows it never built.
