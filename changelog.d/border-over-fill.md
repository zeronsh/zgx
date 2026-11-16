---
section: changed
---
- **A box's fill reaches its outer edge under its border, as CSS's does** (zgx-gpu): the border is painted over the fill (`background-clip: border-box`), so a translucent border shows the fill under it, not what lies under the box. A faint edge on a rounded box had shown its own shadow or the backdrop through it, which read as a dark seam along a dark popover's corners. Opaque borders draw as before. A box with a border width but a fully transparent border colour now fills out to its outer edge, 1 point further than before, as in a browser.
