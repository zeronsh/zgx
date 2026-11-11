---
section: added
---
- **Bounced light in 3D worlds from irradiance probes** (zgx-3d, `gi` feature): `World::add_probes(ProbeVolume::new(min, max, counts))` fills a box with probes that record the light bouncing about the scene (a red wall tints the floor beside it, and a face the lamp can't see is lit by what it sees). Lit meshes inside take their diffuse light from them. Probes are filled by ray queries where the device has them (`unsafe { zgx_gpu::request_ray_queries() }`, before the first window: Metal on macOS 15 and later, Vulkan with ray queries, Direct3D 12 with DXC). The world updates a budget of probes a frame until they converge, then stops drawing. A world without volumes draws exactly as before.
