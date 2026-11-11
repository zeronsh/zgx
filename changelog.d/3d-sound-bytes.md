---
section: added
---
- **A world says what its sound holds for occlusion** (zgx-3d, `audio` feature). `World::sound_bytes()` gives the bytes of the trees over its large meshes' triangles. This is host memory, not the device's, so `World::memory` doesn't count it. A tree goes when its mesh goes or no longer blocks, and a mesh given other triangles gets a new one. A world whose meshes are all small holds none.
