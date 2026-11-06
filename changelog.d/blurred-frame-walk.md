---
section: performance
---
- **A window with backdrop blur walks only what a frame's damage reaches** (zgx-gpu, zgx): a frame of a window with any backdrop blur (frosted glass) walked every node of the window, wherever its damage was, so a row sliding in a sidebar paid for the whole window each frame. The damage is now grown through every filter it could touch first (`Scene::backdrop_filters`) and the walk kept within that, which holds everything the frame draws: the same items, in the same order, and the same pixels. In Zeron's sidebar, a chat's message (its rows sliding, then a working session's glyph turning) costs 53% less drawing on the main thread, 17% less of the process's CPU.
