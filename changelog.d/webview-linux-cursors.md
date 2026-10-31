---
section: fixed
---
- **A web page's cursor on Linux is the one it sets, as it sets it** (zgx-desktop's `webkitgtk`): the cursor was read just after each pointer event, before WebKit's web process had placed the pointer, so it showed the previous one until the next move (a link's hand, after stopping on it, stayed an arrow); and an image cursor (`cursor: url(…)`) showed as the arrow. The window now follows the cursor WebKit sets as it sets it, and shows a page's image cursor itself.
