---
section: added
---
- **Animators take root motion out of a pose** (zgx-3d, feature `gltf`): `World::set_root_motion(animator, Some(RootMotion::new(hips)))` removes a node's motion along the model's ground from its pose and reports it in `World::animator_update`'s `moved` (`.vertical()` its rise too, `.turn()` its turn about up, in `turned`), blended through layers and crossfades as the pose is and measured in the model's frame. `.apply_to(object)` moves and turns that object by it, along its facing, in the same frame.
