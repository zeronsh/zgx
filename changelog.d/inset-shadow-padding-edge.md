---
section: changed
---
- **An inset shadow starts at the padding edge, inside the border** (zgx-gpu): as CSS draws `box-shadow: inset`, so a 0.5 to 1 point ring under a 1 point border no longer vanishes beneath it and design-system elevation tokens port as they are. A box with no border draws as before. Apps that widened an inset ring to show it past a border (a 1.5 ring for a 1 point border) should return to the CSS value.
