---
section: added
---
- **Focus moving into a list scrolls it there, clear of what floats over it** (zgx, zgx-script): Tab, Shift+Tab or `el:focus()` onto a control in a `ui.list`'s row (a transcript's tool row built beyond the rows in view) scrolls the list to it, the offset state following, as a plain scroll view already did. `scroll_padding` (`ui.scroll`, `ui.list`, `View::scroll_padding`; a number or `{ top, bottom, left, right }`) keeps that much clear inside its edges, so a focused row doesn't land under a composer or a title bar. `el:focus({ scroll = false })` (`NodeHandle::focus_with`, `InputDispatcher::focus_in_place`) leaves the views where they are.
