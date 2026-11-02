---
section: added
---
- **`scroll_padding` follows a state** (zgx, zgx-script): `scroll_padding` on `ui.list` and `ui.scroll` takes a state or a function as well as a number or `{ top, bottom, left, right }`, read again as what it reads changes (`View::scroll_padding_with`), so a composer growing to more lines keeps a focused row clear of it, rather than the height it had as the list was made.
