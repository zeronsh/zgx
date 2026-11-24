---
section: performance
---
- **A Luau node with a style that changes keeps less** (zgx-script): its style closure no longer holds a background image and a shader's fill and parameters (two 64-byte blocks) inline, on the many nodes with neither. A Luau table of 10,000 rows in the browser holds 24.5 MB live rather than 27.7, and peaks at 38.4 MB of wasm memory rather than 43.7.
