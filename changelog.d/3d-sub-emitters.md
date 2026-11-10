---
section: added
---
- **Particles give birth to a sub-emitter's where they die or collide** (zgx-3d, `particles` feature): `Emitter::on_death(child, count)` and `on_hit(child, count)` spawn `count` of `child`'s particles at each event, its spawn shape placed there: splashes where rain lands. On the GPU a parent's update writes the births straight into the child's pool; on the CPU the same.
