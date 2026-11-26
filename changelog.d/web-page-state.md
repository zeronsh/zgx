---
section: added
---
- **A web view says which link is under the pointer, the page's theme colour, and each visit** (zgx, zgx-desktop, zgx-script): `WebViewState::hovered_link` and `theme_color` (Luau `web.hovered_link`, `web.theme_color` as `"#rrggbbaa"`), what a browser's status bar and tinted tab show, and `WebViewEvent::Visited { url, title }` (Luau `on_visit`) after each finished navigation, on every engine, what a history records. An engine that doesn't say leaves the field empty: WKWebView has no hovered link, WebView2 no theme colour; Ladybird and WebKitGTK have both.
