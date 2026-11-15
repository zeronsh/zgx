---
section: added
---
- **`ui.window.scale` says how many device pixels a point is** (zgx-script): the display's scale times the window's zoom, read-only and followed, as the web's `devicePixelRatio`. A script, or a plugin, can tell what a canvas's raster costs or which image size to load; before, it could only guess, as no snapshot measures it on Windows or Linux. `ScriptHost::window_zoom` keeps it; `Ui::pixel_scale` is public.
