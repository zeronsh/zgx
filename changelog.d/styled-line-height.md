---
section: changed
---
- **An editor's line its spans shape shorter than the editor's row is its own height** (zgx): in a 16/24 editor, a line spanned at 14/20 (`SpanStyle::size`, `line_height`: a table cell, a small heading) is 20 high, as a web page draws it, and in room of its own (`set_line_insets`) 20 and its room. It was 24: every line was at least the editor's row. Taller lines, empty lines and lines no span shapes are as they were.
