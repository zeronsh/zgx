---
section: fixed
---
- **Popups past the window open on X11 with no window manager** (zgx-desktop): popups past the window wait while the app is at the back, and an X server with no window manager (a headless machine's, a kiosk's, a bare session) never makes a window active, so a menu taller than its window opened in the scene and showed nowhere. An app on such a display now counts as forward (no `_NET_SUPPORTING_WM_CHECK` naming a live window).
- **`zgx inspect windows` lists a popup held while the app is at the back** (zgx-script): such a popup was left out, so a menu open read as `popups: []`. It is listed `held`, with why (`"held": true`, `"why"`); each of `popups` is now a record (`popup`, `held`).
