---
section: fixed
---
- **Occlusion trees are made on one thread, in turn** (zgx-3d, `audio` feature). Every world shares one thread for building the trees over large meshes' triangles. A scene of hundreds of distinct meshes queues their trees on it rather than starting a thread for each. A tree nobody needs by its turn isn't made.
