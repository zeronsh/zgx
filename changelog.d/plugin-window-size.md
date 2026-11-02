---
section: added
---
- **A plugin reads the window's size** (zgx-script): `ui.window.size` is readable in a plugin's VM, read-only and with no grant, so a picture it opens fitted to the window (a lightbox) opens at the window's size, not 1 by 1. The rest of `ui.window` (zoom, visibility, the title bar's area and controls) stays the app's.
