---
section: added
---
- **The clipboard in the DOM** (`zgx-dom`): zgx-dom provides zgx's `Platform` around every view it mounts, so `PlatformService` and Luau's `ui.clipboard` read and write the browser's clipboard, text and HTML both ways. A key a keymap binds to paste reads what is pasted, and `open_url` opens a tab.
