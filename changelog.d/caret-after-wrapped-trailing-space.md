---
section: fixed
---
- **A caret after spaces that end a full wrapped line no longer jumps to the text's start** (zgx-gpu): where a line filled its width and two or more spaces followed, wrapping left the spaces past the width in no line, so the caret after them had no glyph and fell back to the layout's origin. It now stays where the text before it ends.
