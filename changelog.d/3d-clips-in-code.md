---
section: added
---
- **Clips can be made in code** (zgx-3d, feature `gltf`): `Clip::builder(name)` takes channels by node index (`translation`, `rotation`, `scale`, `weight`) of `Keys::step`, `Keys::linear` or `Keys::cubic`, and `build()` refuses keys it can't play (none, not numbers, falling times, values missing, more than `MAX_CLIP_KEYS`) with a `ClipError` rather than panicking. `Placed::new(root, nodes)` (and `with_meshes`) makes objects built in code a model's nodes, for an animator to play them on.
