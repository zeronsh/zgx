---
section: added
---
- **Animators bend their poses by inverse kinematics** (zgx-3d, feature `gltf`): `World::set_ik(animator, hook)` calls `hook` with the blended pose each frame before it's drawn, and `zgx_3d::ik` supplies `two_bone` (an arm or a leg reaching a point, bending toward a pole) and `look_at` (a node's axis turned toward a point, by at most an angle), each with a weight; `Pose` reads and sets nodes' places in their parents' frames and the world. `World::repose` poses an animator again when a target moves while nothing plays. Inputs that aren't numbers leave the pose as it was.
