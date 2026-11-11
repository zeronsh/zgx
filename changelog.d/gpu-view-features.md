---
section: added
---
- **`zgx_gpu::request_view_features()` asks devices for what GPU views can use** (zgx-gpu, zgx-3d): GPU timestamps, block-compressed textures (BC, ASTC, ETC2), indirect draws from a first instance and filtered 32-bit float textures, each where the adapter has it. A device made after the call asks for them; one made without it asks for exactly what it did before. zgx-3d's `World::new` calls it, so a world made before its first window gets them, and `World::caps` reports what the device got.
