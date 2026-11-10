---
section: added
---
- **`ui.debounced(state, ms, { every = true })` tells at each rest** (zgx-script): what reads it hears every time `state` has stayed unchanged for `ms`, even when the value is the one it already held, as when a view's offset is scrolled away and back to where it last rested. Without `every`, a rest at the held value tells nothing, so an app that had to see every rest polled in Luau. With `every`, the wait stays a native timer, and no script runs until the rest.
