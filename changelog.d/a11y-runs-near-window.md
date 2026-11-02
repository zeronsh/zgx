---
section: performance
---
- **A reflow with a screen reader running shapes only the text near the window** (zgx-desktop): each accessibility update shaped every label's text again for its text runs, on screen or not: resizing a 3,000-row list with an assistive client attached cost ~30 ms an update in zgx's projection. Text more than a screen away keeps its runs, shaped again as it comes near; text first seen there gets runs of its whole text placed by estimate, so a reader still gets every character (`AXStringForRange`) and a box near where it is. ~30 ms → 6.6 ms an update on that list; nothing changes with no assistive technology running.
