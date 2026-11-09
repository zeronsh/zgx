---
section: performance
---
- **A column changed at its end places only its end** (zgx): a column that resumed at its end sets aside the children it kept while it is arranged, so they aren't walked again, and takes them back as the layout ends; if it moved, it is arranged whole. Every box is where a whole layout puts it; debug builds check each stands where it is placed.
