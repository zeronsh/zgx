---
section: fixed
---
- **Colour emoji are opaque**: a colour glyph's layers (COLR, as Segoe UI Emoji's on Windows) were composited dividing by 256 where they meant 255, so a fully covered pixel came out at alpha 253: every such emoji showed a little of what was behind it, its colours a little dark. swash is now vendored as `zgx-swash` with that fixed.
