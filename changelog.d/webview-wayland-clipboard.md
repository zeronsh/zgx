---
section: fixed
---
- **A web page's copy reaches other apps whole on Wayland** (zgx-desktop's `webkitgtk`): a selection the page copies goes out with its HTML, so it pastes formatted elsewhere, and an image it copies (its clipboard API, a canvas) as a PNG. It went out as text alone, and an image not at all. The clipboard is now also read after any key, not only one with Control or Shift, as a page's own key handler can copy.
