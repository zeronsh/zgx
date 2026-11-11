---
section: added
---
- **A sky for 3D worlds** (`zgx-3d` feature `media`): `World::set_sky(Some(Sky::new()))` draws the air's light behind the meshes, the sun's disc and a deck of clouds that drift only when asked, in the lights' units; `Sky::sun_light` and `Sky::environment` give the meshes the same sky's sunlight and light all around. A world without one draws as before.
