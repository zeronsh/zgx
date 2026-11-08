---
section: fixed
---
- **A client reading a window's accessibility tree on macOS no longer slows it** (zgx, zgx-desktop): a launcher or window manager was treated as a screen reader, so every name and value went out at once; they now keep to the 100 ms geometry does unless VoiceOver or Switch Control runs, which `NSWorkspace` says live. `Scene::text_lines` walks a node's ancestors once rather than once per grapheme while the scene has a transform, so a slide no longer costs its text's length in walks. A label gone from the scene no longer leaves its runs' place behind in the tree: a window that had shown many rows held one for each.
