---
section: fixed
---
- **A glyph from a fallback font no longer moves the terminal's text off its grid** (zgx, zgx-gpu, zgx-terminal; `zgx-cosmic-text` 0.19.12): a braille spinner or graph (⠋ Loading, btop), a letterlike symbol (℃) or a Thai letter, drawn in a fallback font a little wider or narrower than a cell, pushed every character after it off the grid. A terminal run now takes exactly its characters' cells, each glyph centred in its own, whichever font draws it. `LetterSpacing::cells(width)` asks it of any text: a grid kept in the spacing's own bits, so a font style is no bigger.
