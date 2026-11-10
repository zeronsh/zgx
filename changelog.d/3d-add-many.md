---
section: performance
---
- **Adding objects to a 3D world takes as long for the thousandth as for the first** (zgx-3d): `World::add` looked through every slot before it for a free one, so a world of n objects cost n²/2 checks to build (about 7,000 instructions an add at 2,000 objects). It now keeps where the first free slot can be; slots are reused lowest first, as before.
