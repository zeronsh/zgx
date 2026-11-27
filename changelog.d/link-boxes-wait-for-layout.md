---
section: performance
---
- **A paragraph's links are not shaped for before it has a size** (zgx): a paragraph with a link, mounted as a row scrolls into view, shaped its whole text wrapped at no width for its links' boxes, every one of them then cut away by the paragraph's empty box, before shaping it again once laid out. It now waits for its layout: a long chat transcript scrolled back by a wheel does 35% less work (`transcript-scroll`, x86 instructions).
