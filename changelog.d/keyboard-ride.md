---
section: added
---
- **A scroll view chooses how a focused field clears the iOS keyboard** (`keyboard_ride` on `ui.scroll` and `ui.ios.page`; `View::keyboard_ride` in Rust). `"native"`, the default, scrolls once as UIKit's scroll views do: an ease-in-out over the keyboard animation's own duration, started in its transaction, the field landing on that curve. `"attached"` keeps the field riding the keyboard's edge on the keyboard's spring, the gap between them held, as the kit did before. Off iOS it changes nothing.
