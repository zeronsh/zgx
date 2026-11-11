---
section: added
---
- **A 3D world's emitters change after they're added** (zgx-3d, `particles` feature): `World::set_emitter(id, emitter)` and setters for an emitter's rate, spawn, lifetime, velocity, forces, colour and size and their curves take effect from its next update, its particles living on; a new `max` makes its pool again, held to the device's limits and the memory cap. `World::emitter(id)` gives one back. A lifetime is at most an hour (`MOST_LIFE`) and a size that isn't a finite number is none, so a world always settles.
