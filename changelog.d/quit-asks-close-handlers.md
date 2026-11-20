---
section: changed
---
- **The app menu's Quit (⌘Q) asks every window's `on_close_requested`** (zgx-desktop, macOS): the app quits only if all of them agree, so a quit can hold for an in-flight save. With no handler set, Quit works as before.
