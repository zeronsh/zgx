---
section: added
---
- **Sound in a 3D world** (zgx-3d, `audio` feature): `World::add_emitter3d` plays a zgx-audio sound from an object. It follows the object's place in the world, with distance, cone, equal-power panning, Doppler from frame-to-frame motion, and the air's dulling. The camera (or an object) listens. Meshes between them occlude it through each voice's own low-pass, cast as rays against the triangles picking uses, within a per-frame budget, round-robin, only when an end or a mesh moved; a mesh can let sound through (`World::set_sound_transmission`). Reverb zones fade in as the listener walks toward them. A still world sends no commands.
