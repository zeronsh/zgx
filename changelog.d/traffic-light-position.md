---
section: added
---
- **An app places an inset title bar's traffic lights** (zgx-desktop, zgx-run, macOS): `WindowOptions::traffic_lights: Some((x, y))` puts the close button's top left there, in the content's points from the window's top left, as Electron's `trafficLightPosition`, so a toolbar taller or lower than AppKit's 40-point strip lines its controls up with them; `zgx.json` sets it as `"traffic_lights": { "x": 18, "y": 22 }`. They stay there through resizes, focus and appearance changes and full screen, and move with the window's zoom as the content does. `WindowInfo::controls` and `titlebar_area` (`ui.window.controls`) follow them, the strip as tall as centres them where they sit below AppKit's. Other title bars and platforms ignore it.
