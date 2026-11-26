---
section: added
---
- **A web view takes snapshots of its page** (zgx, zgx-desktop, zgx-script): `WebView::snapshot(area, reply)` (Luau `web:snapshot("visible" | "full_page")`, a promise of a bitmap) gives the page's pixels straight from its engine, without what covers it in the window: what shows, or the whole document. WebView2, WebKitGTK and Ladybird take both; WKWebView takes what shows, and says it can't take the whole page.
