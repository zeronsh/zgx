---
section: fixed
---
- **Images and SVGs land on whole device pixels as the text beside them does** (zgx-gpu): an untransformed image's or SVG's origin is rounded to the device pixel as a text node's is, as Chrome paints them (`ToPixelSnappedRect`). An icon whose top fell on half a device pixel (a line a quarter point off at 2x) drew across two rows, half strength, a row above Chrome's; it now draws whole, and stays the same distance from its text as a scroll moves both by fractions of a pixel.
