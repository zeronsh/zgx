---
section: changed
---
- **macOS draws text as macOS does** (zgx-gpu, zgx-text): Core Text rasterizes every glyph, not only the faces swash can't read, and thickens its strokes by the text's colour as macOS's font smoothing does: all text, light text most (off where the user turned smoothing off). A line of Geist at 2x now puts down within 4% of the coverage AppKit's own text does, light on dark and dark on light; swash's unhinted outlines drew it 26% and 13% lighter, thinner than every other app beside it. Colour glyphs (emoji) stay swash's; Linux and Windows are unchanged. `text::set_font_smoothing` lets an app follow the system's setting (the default) or draw text lighter, for a setting of its own: every window draws its text again.
