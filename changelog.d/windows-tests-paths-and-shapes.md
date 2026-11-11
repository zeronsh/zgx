---
section: fixed
---
- **Tests that failed on every Windows machine pass there, and CI's Windows job runs them** (zgx, zgx-script). A test pasted a temp path into a Luau string, whose backslashes read as escapes. Another joined a path with `/` and compared it with the native one the CLI prints. The reference test wrote a doc example's `(granted …)` note into a file name, so the plugin it shows never loaded on any platform; it now loads, granted. The scroll tests assumed menu items without spacing and scrollbars without arrow buttons, which Windows' have. `luau_pages` runs where Luau's page pool exists (macOS, Linux).
