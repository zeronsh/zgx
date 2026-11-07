---
section: changed
---
- **Drawn glass looks as macOS's own does** (Windows, Linux, and macOS without `NSGlassEffectView`): the material a `glass` box draws where the system has none now makes its face the way the system's glass does, from the numbers its filter holds: what's behind it, its luma moved into a range with its colour kept, worked in Display P3, bent inward within 20 points of its edge, a fill over it, a rim a point wide, and the shadow the regular glass casts in a key window. A tint follows the face's lightness as the system's does. Compared pixel by pixel over solid colours and a black and white edge, light and dark, the mean difference inside the pane fell from 5–40 levels of 255 to 0.1–5 (tinted, from 22 to 3); `scripts/glass_compare.py` measures it.
