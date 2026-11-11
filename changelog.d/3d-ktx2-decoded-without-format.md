---
section: fixed
---
- **A block-compressed KTX2 map draws on every device** (zgx-3d, the `ktx2` feature): where the world's device doesn't hold its format (a world made after `run` without `request_view_features`, ASTC on most desktop GPUs), its blocks are decoded to RGBA8 on the texture threads, where before the map drew as if it had none and its texture was marked failed for every later device too. The decoders are `texture2ddecoder` (MIT), +49.7 KB with the feature on.
