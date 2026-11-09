---
section: fixed
---
- **Hovering a window at the back no longer draws over the app in front** (zgx, zgx-desktop): a tooltip, hover card or menu wanted while none of the app's windows is active stays wanted, and shows as the app comes forward, on every platform; the app's state isn't changed. On macOS showing a popup no longer raises its window, popovers stack at their window's level and tooltips at AppKit's.
