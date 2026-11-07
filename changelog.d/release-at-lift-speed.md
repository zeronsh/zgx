---
section: changed
---
- **A stretch let go while pushing goes on a moment first** (zgx): letting go of a stretched scroll view starts its spring back at the fingers' speed, as an `NSScrollView` does, measured: pushed at 1500 points a second it goes about 2 points further over 15 ms before coming back. Fingers rested 200 ms first let go from rest. A momentum's bounce ends after its furthest point, as AppKit's does, not after a fixed 24 ms.
