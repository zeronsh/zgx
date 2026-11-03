---
section: fixed
---
- **An inset title bar keeps its strip when going into full screen fails** (zgx-desktop, macOS): the inset title bar's toolbar is hidden as a window goes into full screen, and a transition that failed after it began (AppKit tells only the window's delegate) left it hidden on a window that wasn't in full screen, its traffic lights no longer centred. Two seconds after a window begins going into full screen, one that isn't gets its toolbar back.
