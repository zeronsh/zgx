---
section: performance
---
- **A flood of plain text parses about a third faster** (zgx-terminal): a run of printable text is found eight bytes at a time rather than one, written over its cells in one pass (nothing mapped or cloned for plain ASCII with no hyperlink or underline colour), and a line scrolled into the history with nothing extra in its cells is cleared field by field. Printing a coloured 20 MB log went from 540M instructions to 378M in the parser, and from 185M to 163M clearing lines; the gate's `terminal` and `terminal-reader` scenes count 3% fewer.
