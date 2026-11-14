---
section: added
---
- **`ui.render_canvas(draw, { width, height, scale, x, y, background })` gives a drawing as a PNG** (zgx-script, zgx-gpu): `draw(c, w, h)` runs with the `c` a canvas's `draw` gets, and the promise gives what that canvas shows, in base64. Its shapes are rasterized as a canvas's are. Its text is shaped in the system's fonts and the app's own, as its windows shape it (`Platform::fonts`, which zgx-desktop answers with `Application::font`'s and `replace_font`'s), and drawn over the shapes. No window has to show it, so a plugin can picture a design for an agent or an export while the app is hidden. In Rust: `zgx_gpu::canvas::picture` and `canvas::png`.
