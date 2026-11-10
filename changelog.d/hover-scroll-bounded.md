---
section: performance
---
- **Content scrolling under a still pointer finds the row under it without walking every row's parts** (zgx): the hit test that keeps hover right each scroll frame skips a row the pointer isn't over by the proof its box keeps of which sides its subtree reaches past, wherever the scroll has moved it. Before, the proof was used only for nodes at rest in a scene without raised or sticky children or popups, so a scroll's rows were all entered. In a list of 1,000 five-node rows, a scroll frame's hit test visits at most 1,016 nodes instead of 6,004, with or without a raised node in the window.
