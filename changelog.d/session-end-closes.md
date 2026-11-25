---
section: fixed
---
- **A quit inside an app's handler no longer ends it without its `on_closed`** (zgx-desktop, macOS): AppKit's `terminate:` called in a handler, ⌘Q before the menu bar was made among them, aborted the process. It now quits once the handler returns. Each `on_closed` handler runs once whichever way its window goes, and one that panics stops no other.
