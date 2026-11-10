---
section: performance
---
- **A canvas text of known size is never measured** (zgx, zgx-terminal): `CanvasText::extent` gives a text's size when its maker knows one at least as large, and its label is laid out and damaged by it without measuring (`Scene::set_text_measured`). A terminal's rows give theirs: about 9% of a terminal frame's work was measuring text whose size the grid already knew. A canvas text without one is measured as before.
