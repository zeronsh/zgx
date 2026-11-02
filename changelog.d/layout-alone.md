---
section: performance
---
- **A change a row or column's child keeps to itself lays out that child alone** (zgx): a text changing inside a child its row or column asked one size of (a fixed-width list's rows, a document's blocks) used to lay out the whole row or column and every ancestor; the child is now laid out again on its own and, if it answers as it did, its siblings stay as they are. A streaming Markdown append with 256 paragraphs above it costs 30% fewer instructions (7% with one), and `update_bench`'s rows updated in place 21% fewer. A change that does move its siblings lays out as before.
