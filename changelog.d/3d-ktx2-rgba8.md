---
section: added
---
- **KTX2 textures, their levels as stored** (zgx-3d, the `ktx2` feature): `Texture::decode` reads KTX2 files of RGBA8 texels (sRGB or not, not supercompressed), putting their levels on the GPU as they are, with no decoding and the file's own smaller levels; a glTF texture's `KHR_texture_basisu` image is read. Block-compressed and Basis Universal KTX2 files are refused with the reason, until they're read too.
