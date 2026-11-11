---
section: added
---
- **`ui.rich { spans = function() … end }`: a paragraph whose runs follow state** (zgx-script). The function returns the runs, as `ui.rich` lists them, and is called again in place when what it reads changes: the paragraph keeps its node, its selection and its text layout (a paragraph that only grew keeps its shaped lines, glyphs and quads). A paragraph whose runs came and went with a state was built again, a new node each time, so a streamed reply's paragraph laid out and drew its whole text again every chunk.
