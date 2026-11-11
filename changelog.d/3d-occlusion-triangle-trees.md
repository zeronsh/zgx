---
section: changed
---
- **Sound occlusion meets large meshes through a tree of their triangles** (zgx-3d, `audio` feature). A mesh of more than 256 triangles that blocks sound now gets a tree over its triangles. The tree is made once per geometry on a thread of its own, and until it's ready the mesh is tested triangle by triangle as before. A ray tests only the boxes and triangles along it, and stops at the first triangle it meets. In the harbor's sound with a blocking ground of 320,000 triangles, the world's sound costs 16 µs a frame (median) instead of about 2 ms, with the same answers. The tree takes about 50 ms to make and holds 5.5 MB.
