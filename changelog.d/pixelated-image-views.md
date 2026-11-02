---
section: fixed
---
- **`image_rendering = "pixelated"` draws an image view's picture pixelated** (zgx): the setting landed on the image view's box, while its picture is a node inside it, which the renderer asked. Every `ui.image` drew smooth (Zeron's lightbox at 300%: a 1 px checker ramped 46 → 202 → 46). `image_rendering` is now inherited, as CSS's `image-rendering` is: a node's own setting, else the nearest one around it.
