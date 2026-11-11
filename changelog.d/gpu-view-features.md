---
section: added
---
- **Apps can ask devices for what GPU views can use** (zgx-gpu, zgx-3d): `request_view_features()` (from zgx-gpu, or re-exported by zgx-3d), called before `Application::run`, makes devices ask for GPU timestamps, block-compressed textures (BC, ASTC, ETC2), indirect draws from a first instance and filtered 32-bit float textures, each where the adapter has it. `run` decides before it starts the device, so a later request changes nothing, the same on every platform and launch. Not called, the device request is exactly what it was. `World::caps` reports what the device got; a debug build says once when a world is made after `run` decided without them.
