---
section: performance
---
- **Text inputs a width change reached lay out once, however many**: each text input whose width changed ran a layout pass of its own, from the root, for what the input before it had just dirtied: N inputs, N + 1 passes a frame while a window resized (9 at 8 inputs). They now draw from the frame's layout, and the frame's next pass lays out what they dirtied: 2 passes a frame at any number of inputs.
