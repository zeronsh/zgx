---
section: fixed
---
- **Hebrew and Arabic fall back to SF Hebrew and SF Arabic on macOS** (zgx-cosmic-text): the system font's fallback in Core Text's cascade and Chrome, hidden variable faces (`.SF Hebrew`, `.SF Arabic`), before Arial and Geeza Pro, so medium and in-between weights (450, 500) draw at their own axis values in one face, where Geeza Pro had no Medium. Tested on a Mac with the faces: widths at 400, 450 and 500 increase.
- **A scroll view capped by `max_h` isn't flagged as overflowing without scrolling** (zgx): the layout lint counted only a fixed-size scroll view as scrolling.
