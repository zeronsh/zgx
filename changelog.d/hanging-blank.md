---
section: fixed
---
- **A wrapping text's blank past its width hangs, as CSS's trailing spaces do**: a text ending in spaces measured wider than it was given, and a box that took that width broke it into fewer lines than it measured. Its width now leaves out the one blank a line takes past the width, so it measures within that width and keeps its lines; the blank is still laid out where it was, so a caret after it stays there. A right-aligned or centred line ending in such a blank now sits where its words put it, as in CSS, rather than at the left.
