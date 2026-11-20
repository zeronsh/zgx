---
section: added
---
- **Text in a right to left node takes the right to left base level** (zgx, zgx-gpu, zgx-cosmic-text): a paragraph in a node running right to left (`dir = "rtl"`, or inherited, or `dir = "auto"` on Arabic) shapes at that base level whatever its first strong character is, as CSS's `direction: rtl` does, so Latin text in an Arabic answer places its closing punctuation at the left as Chrome's right to left paragraph does. `FontStyle::paragraph_rtl` (set by the scene), `Attrs::paragraph_rtl`, `RichText::with_paragraph_rtl`. Left to right nodes keep the first strong character's way.
