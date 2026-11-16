---
section: performance
---
- **A canvas's text keeps no copy of each line** (zgx): a canvas's labels compare each line with the last drawing's, which the canvas's node holds anyway, instead of keeping a clone of every line (its text and font, four shared counts each) to compare with. A terminal's changed run costs about seven fewer atomic counts; a log streaming in takes 0.5% fewer instructions, and each canvas holds one drawing where it held a copy of every line.
