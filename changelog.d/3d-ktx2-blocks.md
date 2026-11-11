---
section: added
---
- **KTX2 textures of BC, ETC2, EAC and ASTC blocks, and Zstandard-supercompressed ones** (zgx-3d, the `ktx2` feature): their levels go to the GPU as stored, where the world's device holds their format (`World::caps().compressed`), else the map draws as if it had none. zgx-gpu gains `viewport::texture_with_block_levels`, which writes a block-compressed texture's levels in place on Metal with unified memory (no blit) and with `write_texture` elsewhere.
