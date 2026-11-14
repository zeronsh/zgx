---
section: added
---
- **A canvas draws pictures** (zgx-script, zgx, zgx-gpu): `c:image(src, x, y, w, h, { fit, radius, opacity })` draws a file, a URL, a `host:` image or a `Bitmap` among the canvas's shapes, in the order drawn, fitted to its box as `ui.image` fits one (`"cover"` crops it), cut to the box's rounded corners and the canvas's clip. A remote or `host:` picture not there yet draws nothing until it comes, and the drawing follows it. In Rust, `Canvas::image` and `Draw::Image`. A canvas with a picture rasterizes, as one with a gradient does.
