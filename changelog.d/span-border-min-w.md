---
section: added
---
- **Inline outlined boxes: `ui.span { border = { width, color }, min_w }`** (zgx, zgx-script, zgx-gpu, zgx-cosmic-text): a span takes a border (CSS's inline border: it takes room all round and is drawn inside the span's box, following its `radius`, over its background or alone) and a least width (`min_w`, border-box: what room it lacks goes half before the text and half after, so the text sits centred), so a keycap is one span in a paragraph that wraps as one. The color may be a token. `TextSpan::border` and `min_width` for Rust callers; the box clips at its paragraph's edge as backgrounds with vertical padding do. A test draws the outline, its room and its centring at 2x.
