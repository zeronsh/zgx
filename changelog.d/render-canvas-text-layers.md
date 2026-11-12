---
section: fixed
---
- **A rendered canvas draws its text in the window's layers** (zgx-gpu): `ui.render_canvas` drew every text over all of a canvas's shapes, uncut by its clips and upright, while a window draws each text in its layer, under shapes drawn after it, cut by its clip's box and turned by its transform. A picture now paints each layer's shapes and then its text, cuts a text by its clip's box and draws a turned text mapped by its transform, so it matches the window: a mean channel difference of 0.007 at 1x and 0.002 at 2x on Direct3D 12, none over 32 (4.7 before).
