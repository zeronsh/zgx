---
section: fixed
---
- **A box with fractional edges is anti-aliased on every side** (zgx-gpu): a quad is shaded only at the pixels whose centres it covers, so an edge in the outer half of a pixel (a box starting 0.7 into one) lost that pixel's coverage altogether and an edge in the inner half kept it: a box was half snapped to whole pixels, half anti-aliased (its 1 px ring a pixel off its fill). Box, shadow-ring and outline quads now reach the pixel boundaries outside their box (kept-frame patching follows), and the shader covers the extra pixels as the edge does.
