---
section: added
---
- **`zgx inspect windows` says what shows each popup on X11** (zgx-desktop, zgx-script): a popup in a window of its own lists its surface's composite alpha mode, whether it presents opaque and whether zgx found a compositor (`alpha`, `opaque`, `compositor` in its record), so a capture whose popup corners or shadow margin read black can tell why without a restart.
