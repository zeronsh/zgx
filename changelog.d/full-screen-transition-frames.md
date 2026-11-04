---
section: fixed
---
- **A window keeps drawing through a full-screen transition, and says where its title bar's buttons end up** (zgx-desktop, macOS): AppKit counts a window covered while it animates its snapshot of it, and zgx stopped drawing for those 0.55 s, with `ui.window.shown` false meanwhile; now it goes on, and counts the window covered only if it still is once the transition is over. Through a transition `ui.window.controls` and `titlebar_area` hold where they end up (nil going in; their place going out), never AppKit's place partway. `full_screen_transition`'s `duration` is as long as the last one that way took, about 0.55 s at first, where AppKit's resize estimate said 0.35 s in and 0.07 s out.
