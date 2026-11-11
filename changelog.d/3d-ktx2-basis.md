---
section: added
---
- **Basis Universal textures** (zgx-3d, the opt-in `ktx2-basis` feature): KTX2 files of ETC1S or UASTC, and glTF images given by `KHR_texture_basisu`, are transcoded on the texture threads to BC7 or ASTC 4x4 where the world's device holds them, RGBA8 otherwise, with the file's own levels. The transcoder is the pure-Rust `basisu` crate (Apache-2.0; no C or C++): an app turning the feature on ships its code and should list its licence among its third-party notices. About +790 KB to a stripped release binary with it on; nothing without.
