---
section: fixed
---
- **An accessibility update between frames no longer reads a removed node** (zgx-desktop): with an assistive client attached, an update held back for pacing could fall due after a handler unmounted part of the window (a sidebar closing) and before the frame that shows it, and fail with "stale scene node" or tell the client an old tree. It now looks at the whole tree then.
