---
section: added
---
- **A 3D world's frame can go through a post chain, starting with bloom** (zgx-3d, `post` feature): `World::set_post(Post { bloom: Some(Bloom::default()), .. })` draws the meshes in linear light into a float target and maps their tones in a last pass, with bloom mixed in: the scene blurred wider and wider (13-tap steps down with Karis's average on the first, tent steps up) and mixed rather than added, so it brings no light of its own. `Vignette`, `Grain` (a function of the frame's index, so recorded frames repeat) and `ChromaticAberration` are compiled into the last pass when on. A world with nothing on draws exactly as before.
