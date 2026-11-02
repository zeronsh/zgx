---
section: added
---
- **A web page's full screen** (zgx `WebViewState::full_screen`, `WebView::exit_full_screen`; Luau `web.full_screen`, `web:exit_full_screen()`): a page's `requestFullscreen` is told to the app, tracked like the page's other state, on every platform. On macOS it now works, in WebKit's own window over the screen (WKWebView's element full screen was off, so pages' requests failed); on Windows and Linux the element fills the page's node, which an app can grow, or make its window full screen, to show it so.
