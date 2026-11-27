---
section: added
---
- **A web view takes a browser's Edit menu, reloads ignoring its cache, and says when its page hangs or why it ended** (zgx, zgx-desktop, zgx-script): `WebView::edit(EditCommand)` (Luau `web:edit("copy" | "cut" | "paste" | "select_all" | "undo" | "redo")`) through the system's clipboard, `reload_ignoring_cache()` (Shift+reload), `WebViewEvent::Unresponsive`/`Responsive` with `WebViewState::unresponsive`, and `WebViewState::termination` (crashed, out of memory, killed) beside `ProcessTerminated`. Ladybird pages now copy to and paste from the system's clipboard. On Windows a page that stops answering now says `Unresponsive` rather than `ProcessTerminated`: its process still runs.
