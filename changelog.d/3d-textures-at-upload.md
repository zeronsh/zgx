---
section: performance
---
- **A textured 3D model loads in a third of the time and keeps 220 MB less of 64 textures** (zgx-3d): `Texture::decode` reads a PNG's or JPEG's header and keeps the file, its pixels decoded as a renderer uploads it, on up to 4 threads, and dropped once it has; a model no longer holds every texture decoded at once, which left a large block per texture in the process's footprint for good. 64 textures of 1024² (M5 Max): file to first frame 420 → 141 ms, heap peak 364 → 145 MB, footprint at rest 750–790 → 578 MB. A file whose header reads but whose pixels don't decode draws as if it had no texture.
