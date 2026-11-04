---
section: added
---
- **An app can ask whether the system blurs behind its glass, and whether its window is maximized** (zgx-desktop, zgx-script): `WindowCapabilities::blur_behind` is false on Linux where the compositor has no blur protocol (GNOME, sway, Weston, and X11, where zgx asks for none), where glass shows what is behind it sharp, so an app may draw solid; `WindowContext::maximized`, and in scripts `ui.window.maximized` (`ScriptHost::window_maximized`), follow the system's word on the window.
