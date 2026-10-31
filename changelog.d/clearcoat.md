---
section: added
---
- **Clear coats in 3D scenes** (zgx-3d): `World::set_clearcoat(id, Some(Clearcoat::new(1.).roughness(0.1)))` lacquers a mesh, as car paint, varnish or a glaze: a thin coat of glass reflects the lights and surroundings over it, smooth or rough, and what is under it (light through glass included) shows the less the more the coat reflects. glTF models bring theirs: `KHR_materials_clearcoat`'s factor and roughness (not yet its textures). A mesh with one draws with a pipeline of its own; a world with none binds nothing new.
