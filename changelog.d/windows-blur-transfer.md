---
section: added
---
- **`WindowCapabilities::blur_pass` and `blur_lift`** (zgx-desktop): how much of what is behind the window its system blur lets through, and what it adds. Windows' acrylic lets 83% through and adds 8. macOS and Linux report a pure blur. An app can use them to pick a scrim that comes out the same on every platform.
