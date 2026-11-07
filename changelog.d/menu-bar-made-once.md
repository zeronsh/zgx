---
section: performance
---
- **A mouse move no longer has AppKit make the menu bar again** (macOS): with the menu bar set to hide itself, an app that activated before its first menu bar was in (most launches: zgx sets its bar after the first frame) had AppKit make the bar's windows again for every move, about 0.3 ms of the app's CPU a move on every page. The host now has AppKit settle the bar once its first one is in.
