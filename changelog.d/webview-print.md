---
section: added
---
- **`WebView::print`** (zgx): the platform's print dialog for a web page, as the page's own `window.print()` shows it. On macOS both are now the print panel as a sheet on the window (the page's `window.print()` did nothing); on Windows WebView2's print preview; on Linux GTK's print dialog, now transient for the app's window on X11 (it opened wherever the window manager put it, owned by no window it knew).
