---
section: fixed
---
- **Nothing a scroll leaves behind stays on screen**: scroll content that repaints rather than copies when it moves (a layer of its own, such as a faded or isolated view) repaints what goes or is laid out again in it after the move, where it was drawn before the move. A node removed in such a frame (a reply's hover lane under a still pointer, in a measured list flung past it) was damaged only where the content had moved it, and its pixels stayed where they were drawn until something painted there again.
