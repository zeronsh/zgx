---
section: fixed
---
- **The caret after spaces that hang past a full line stays at that line's end** (zgx): a word that filled its line, then two spaces, left the caret drawn at the text's start while typing went on at its end (the second space has no glyph once it wraps, and the caret fell back to the layout's origin). It now stays after the line's last glyph, where the spaces hang, as a Mac text view keeps it.
