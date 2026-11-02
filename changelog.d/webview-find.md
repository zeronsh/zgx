---
section: added
---
- **`WebView::find`** (zgx, with `FindOptions` and `FindResult`, and `WebView::clear_find`): find text in a web page as a browser's find bar does, on macOS, Windows and Linux. The next match (or the previous) is selected and scrolled to, wrapping past the end, and the reply says whether there is one and, on Windows and Linux, how many. Asking again with the same text steps to the next match.
