---
section: fixed
---
- **`w = "fit-content"` on a box whose text wraps takes its room, as CSS's does** (zgx): its max-content was measured with its own `max_w` still on, so its text wrapped first and the box came out its widest wrapped line, the same as with no width. A chat bubble capped at 70% now reaches the cap (401.8 in a 574 column, where it was 393.6), as in a browser. A box with no width still shrinks to its widest line.
