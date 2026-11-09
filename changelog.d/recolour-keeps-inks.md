---
section: performance
---
- **Recolouring rich text keeps its ancestors' cached inks** (zgx): a text whose runs change colour alone (a fade's frame) marks its paint dirty and keeps the inks and bounds of the nodes that hold it, so the frame's damaged walk no longer works them out again up the whole chain. Any other change to the text clears them as before.
