---
section: performance
---
- **A large glTF model reads faster** (zgx-3d): `Model::from_bytes_with` builds a scene's meshes on several threads (up to 8; a model of fewer than 64 meshes on its own), and keeps each PNG's or JPEG's file as the resolver gave it, its size read from its header, rather than copying it. Worlds also take turns on the texture threads, so one world's first map doesn't wait behind another's hundreds, and maps load in the frame where no thread can be started.
