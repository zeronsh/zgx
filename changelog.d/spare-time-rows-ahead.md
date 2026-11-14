---
section: performance
---
- **Rows a list builds in spare time leave observers to the frame** (zgx): each row a list builds ahead of its view in a refresh's spare time was laid out by a frame's preparation, which gathered and published every observed node's bounds and visibility too, some 47 times a second while scrolling. It is now a preparation ahead of the frame's, as dispatching input is: rows are built and laid out, and the frame's own preparation delivers observations once before it draws. On an M5 at 120 Hz with a transcript's rows observed, a scroll's CPU a frame went down about 9%.
