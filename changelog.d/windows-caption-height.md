---
section: added
---
- **`WindowOptions::caption_height`** (zgx-desktop; `caption_height` in `zgx.json`): how tall `Titlebar::Inset`'s caption buttons are on Windows, in the content's points, so they span an app's title bar at any zoom. Drawing, hit-testing and `WindowInfo::controls` all use it. Unset, they stay 32 points of the display.
