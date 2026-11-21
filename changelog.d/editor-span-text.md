---
section: added
---
- **A text area's spans take a size, a line pitch, an underline and the monospaced font** (zgx, zgx-script): a highlighter's span can set `size`, `line_height`, `underline` and `font = "mono"` (`SpanStyle::size`, `line_height`, `underline`, `mono`), so a note's headings, links and inline code show in one editor. Size, pitch and font shape the line, so carets, clicks and wrapping follow them. Spans without them stay as small as before: a span names its size, pitch and font in two bytes, each different one kept once (`SpanText`).
