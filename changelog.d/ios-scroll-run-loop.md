---
section: fixed
---
- **On iOS an app's timers keep running while a finger drags or a scroll view coasts** (iOS; winit only). `ui.every`, `ui.after` and a live feed's inserts waited for a scroll to end, and frames stay paced meanwhile. A status bar tap scrolls to the top with the keyboard up, and a list the engine anchors (rows inserted above kept in view) stays put as UIKit's scroller goes on.
