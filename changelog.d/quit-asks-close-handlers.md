---
section: changed
---
- **The app menu's Quit (⌘Q) asks every window's `on_close_requested`** (zgx-desktop, macOS): the key window's first, then the rest as they opened; the first refusal holds the quit, which no window's handler could before. With no handler set, Quit works as it did. `WindowFactory::quit` asks none. A logout, a restart or a shutdown isn't held: it asks nothing and runs each `on_closed`.
