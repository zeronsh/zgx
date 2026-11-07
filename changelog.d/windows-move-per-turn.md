---
section: performance
---
- **A pane's edge dragged on Windows follows the pointer at the display's rate** (zgx-desktop): a turn of the event loop handles its pointer moves as one, the last, as on Linux. Windows folds only the moves not yet read, so with a 1000 Hz mouse a move that relaid the window out (a pane's edge dragged) found the next already waiting: one turn laid the window out for each, 11 times in one frame, and paints waited, up to 0.8 s, until the pointer stopped.
