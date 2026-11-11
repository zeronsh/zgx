---
section: added
---
- **A 3D mesh can stay out of the sun's shadow** (zgx-3d, `lighting` feature): `World::set_casts_shadows(id, false)` leaves it out of the sun's shadow map or cascades, and `World::set_receives_shadows(id, false)` has the sun light it wherever a shadow would fall on it. Both are on by default.
