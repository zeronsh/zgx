---
section: added
---
- **An editor's lines can share a row** (`LineInset::joins_next`, `LineInset::width`): a line and the next sit side by side (a table's cells), each wrapped in its own width at its `left`; the row is as tall as its tallest line, and a `LineMark` of infinite height or width fills its row and its cell. `LineInset` has two new fields: build it with `..LineInset::default()`.
