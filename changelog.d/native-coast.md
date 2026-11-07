---
section: changed
---
- **A trackpad scroll coasts as an `NSScrollView` does** (zgx-desktop, macOS): after the fingers lift, a scroll goes on from their speed, measured as AppKit measures it, on AppKit's own curve: a quartic ease-out that ends in a finite time, moving in whole points, each refresh showing where it will be when that refresh reaches the screen. The system's momentum events go unused, as AppKit's scroll views leave them. Under the same posted flicks, the speed zgx lifts at equals what AppKit's measure gives over the same events' timestamps, in 9 of 9; the coast follows from it as AppKit's does.
