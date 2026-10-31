---
section: added
---
- **Drags out of a web page on Linux** (zgx-desktop's `webkitgtk`): a page's link, image or selected text drags as zgx's own drag. It drops on zgx's targets as `DraggedText` (the selection, or the address), and out of the window on other apps over XDND or Wayland's data device: the address, the text, and an image's bytes from WebKit's own copy, so a page behind a login drags its images too. A selection goes as plain text.
