---
section: fixed
---
- **An outline made dotted, dashed or solid repaints at once** (zgx): `Scene::set_outline_style` changed alone (no other change to the box in that frame) marked the box painted but added no damage, so the old outline stayed on screen until something else repainted there.
