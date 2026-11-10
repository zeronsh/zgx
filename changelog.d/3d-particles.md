---
section: added
---
- **3D worlds have particle emitters** (zgx-3d, `particles` feature): `World::add_emitter` takes an `Emitter` (spawn shapes, rate and bursts, lifetime, velocity, gravity, drag, wind and swirl, plane collision, billboards or streaks blended over or added, colour and size over life). Particles update in a compute pass, or on the CPU (up to 16,384 an emitter) where the device has no compute. A world draws while particles may live and settles when none can. Particles draw in front of meshes until the renderer's pass list draws them against depth.
