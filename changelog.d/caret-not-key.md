---
section: fixed
---
- **A window that is not key shows no caret and wakes for no blink** (zgx): the focused editor's caret kept blinking, drawing a frame a phase, in a window that was never key (an offscreen one: 60 of 68 frames in 30 s) or sent to the back; AppKit draws none there. The caret now hides when the window stops being active, wakes for nothing while it is not, and shows solid and blinks again as it becomes key. The desktop host takes the window's focus at creation rather than assuming it is active. A test covers the hide, the silence and the return.
