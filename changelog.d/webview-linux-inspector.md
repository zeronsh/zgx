---
section: fixed
---
- **Web Inspector opens in a window of its own on Linux, however large the page** (zgx-desktop's `webkitgtk`): WebKitGTK docked the inspector inside a large enough page, where it drew in the page's frames but heard none of its input. It is now detached as WebKit docks it, as its own undock button would, before the page lays out around it.
