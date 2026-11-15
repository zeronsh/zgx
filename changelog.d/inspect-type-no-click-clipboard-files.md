---
section: added
---
- **`zgx inspect type --no-click` and `clipboard set --files`** (zgx-script): `type --no-click <text>` types where focus is, its caret and selection left as they are (a click first would move them); `clipboard set --files a.pdf b.png` puts files on an app's own clipboard (`ZGX_CLIPBOARD=memory`) as a file manager copies them, so `key cmd+v` pastes them into an `on_paste` as files, and `clipboard` reads them back on a `files:` line. Both are the MCP tools' too (`no_click`, `files`).
