---
section: changed
---
- **`text::rich_decorations` and `text::rich_decorations_in` take the fonts** (zgx-gpu, and zgx-text, where they are now): a last argument, `fonts: &mut FontSystem`, the one the text was shaped in: `rich_decorations(&buffer, &rich, &mut fonts)`. zgx-gpu's other text paths are where they were.
