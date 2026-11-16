---
section: performance
---
- **A pointer moving beside a list of rows with icons no longer walks every row**: the hit test skips a subtree the point is outside once nothing in it can be hit past that side, and an image or SVG a view mounts drawn in its box (an icon) no longer stops that proof; one transformed past its box, or appended to the scene directly, still does. A sidebar of 500 rows with icons: 64 nodes walked a move instead of 2,064.
