---
section: added
---
- **Specular and index of refraction in 3D scenes** (zgx-3d): `World::set_specular(id, Some(Specular::new(1.).color(tint).ior(1.7)))` sets how much a dielectric mesh reflects, tinted how: as much facing it as its index of refraction says (water a little, diamond a lot), that and its grazing reflection scaled by its factor. glTF models bring `KHR_materials_specular`'s factor and colour (not yet its textures) and `KHR_materials_ior`'s index for any material, not glass's alone. A mesh reflecting otherwise than any does draws with a pipeline of its own; a world with none binds nothing new.
