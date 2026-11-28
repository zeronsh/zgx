---
section: performance
---
- **A terminal paint holds no row past its slot** (zgx-terminal): an old row's paint goes as its slot takes the new one, not with a list of every row shown at the end of the paint, whatever the compiler inlines (terminal cells up to -0.47% instructions).
