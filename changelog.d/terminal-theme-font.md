---
section: fixed
---
- **The terminal follows the theme's monospaced font**: the engine-drawn terminal drew its grid and its composition in the platform's monospaced font whatever the theme said. It takes `font.mono` from the theme scope its view mounts in, and follows it as it changes (the cell measured again, every row made again). Where the theme names none, it keeps the platform's monospaced font. The Ghostty backend on macOS keeps its own font settings. Thanks to @katulevskiy.
