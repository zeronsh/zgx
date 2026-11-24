---
section: fixed
---
- **A text area's chips are one drawing, under the selection** (zgx): a span a highlighter styles as a chip (`SpanStyle::chip`) is drawn as an atom's wash is, a mark behind the text, so selecting it shows the selection over it (its background used to hide it). Its padding across still takes room in the line, added to an atom's leading and trailing room where both apply. Each chip and atom has its own corners (they used to share one radius per text area, which a search's matches took too); search matches keep the theme's. Existing atoms draw exactly as before. A chip span's height now follows an atom's (`size × 1.16` plus its padding down, centred in its row), which can differ from the font's content area by about a point.
