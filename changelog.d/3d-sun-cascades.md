---
section: added
---
- **A 3D sun's shadow in up to four cascades** (zgx-3d, `lighting` feature): `World::set_cascades(Some(Cascades::new(4)))` gives the view's near slices sharp shadows and its far ones wide ones, each a 16-bit map of its own; they move by whole texels so edges don't swim, and each is drawn again only when it moved, the sun did or a caster inside it did, the farther ones following a moving camera one a frame.
