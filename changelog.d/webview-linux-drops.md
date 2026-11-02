---
section: added
---
- **Drops into web pages on Linux** (zgx-desktop, `webkitgtk`): a page's own HTML drags now drop within it (its `dragstart` data, any types), zgx's dragged text drops on it, and files from other apps drop as `File`s with their bytes, each as the page's `dragenter`, `dragover`, `dragleave` and `drop` where the pointer is. Before, a page's own drag left it for zgx's, and nothing dropped on a page reached it.
