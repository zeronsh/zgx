---
section: added
---
- **A web page's downloads are the app's to save** (zgx `WebView::set_download_handler`, `DownloadRequest`, `DownloadResponder`, `DownloadStatus`; Luau `ui.webview { on_download = function(d) … end }` with `d.url`, `d.name`, `d.mime`, `d:save_to(file)`, `d:ask()` and `d:cancel()`): each download a page starts (a `download` link, an attachment, a response it can't show) reaches the app, which saves it at a file the person chose, as through its save dialog, or not, and hears its progress and its end, on macOS (`WKDownload`), Windows (WebView2, its download bar hidden) and Linux (`WebKitDownload`). With no handler nothing is saved, as before: a page never saves into the person's folders without the app deciding.
