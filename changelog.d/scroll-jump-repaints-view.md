---
section: fixed
---
- **A scroll that jumps too far to copy no longer leaves behind what went in the same frame** (zgx): a move that can't copy its pixels repaints all its view shows. Before, it repainted what its content held, at the old offset and the new, plus earlier damage shifted back by the move. A box removed after the content moved was damaged at the new offset, out of the window, and so was dropped, while the walk at the old offset no longer held it. Zeron left a turn's text on screen this way when its transcript jumped as the turn's rows were rebuilt.
