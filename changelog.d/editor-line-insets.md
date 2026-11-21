---
section: added
---
- **A text area's lines can take room of their own** (zgx): `EditorHandle::set_line_insets` gives each line an indent and space above and below (`LineInset`), so a note's list items and headings sit as they do on the web. A line wraps in what its indent leaves, and its carets, hits, selection and the lines below follow. An editor without insets keeps none, and lays out and scrolls as before.
