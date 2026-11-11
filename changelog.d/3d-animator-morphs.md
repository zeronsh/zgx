---
section: added
---
- **Animators play morph weights through their layers** (zgx-3d, feature `gltf`): a clip's weight channels crossfade, blend by layer weight and mask, and add (`Layer::additive()`) as nodes' places do, written to the node's meshes only when they change. A crossfade's or a stop's fade now begins with the frame after it's asked for, as the new clip does: the clips fading out no longer lose weight to the rest pose meanwhile.
