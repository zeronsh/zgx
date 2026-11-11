---
section: added
---
- **Each mesh has four numbers of its own for its custom material** (zgx-3d, `shaders` feature): `World::set_instance_floats(mesh, Some([a, b, c, d]))`, read as `VertexIn.instance` and `SurfaceIn.instance` in `displace`, `surface` and `opacity`, in the meshes' pass and the shadow's, so meshes of one shader vary (a colour, a wave's phase, wear) with no binding or shader of their own. A world whose meshes have none writes nothing for them.
