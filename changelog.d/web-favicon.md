---
section: added
---
- **A web view keeps its page's icon** (zgx, zgx-desktop, zgx-script): with `WebViewOptions::favicons` (Luau `favicons = true`), `WebViewState::favicon` (Luau `web.favicon`, a bitmap) holds the page's icon as its engine loaded it, and `WebViewEvent::FaviconChanged` says when it changes, as a browser's tab shows it. WebView2, WebKitGTK (its favicon database turned on for views that ask) and Ladybird give it; WKWebView doesn't, and leaves it empty.
