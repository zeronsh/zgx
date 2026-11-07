---
section: changed
---
- **A mouse wheel's notch moves 30 points at once on macOS, as in AppKit's views** (zgx-desktop): each notch moves its whole step on the next refresh, notches within one refresh together, at an `NSScrollView`'s line of 10 points (3 lines, 30 points, a notch at the system's default). It moved a browser's 120 points, eased in over about 150 ms. Windows and Linux are unchanged.
