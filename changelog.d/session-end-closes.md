---
section: fixed
---
- **A logout, a restart or a shutdown isn't cancelled by an app with an `on_close_requested`** (zgx-desktop, macOS): every quit waited for the handlers, so the session never ended. One that ends the session (the quit event's `kAEQuitReason`) asks nothing: each window's `on_closed` runs, once, before the quit is let go on, and one that panics stops no other.
