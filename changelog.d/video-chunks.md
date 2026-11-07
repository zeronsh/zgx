---
section: performance
---
- **A video renders in chunks at once** (`zgx-video`): `zgx video render --concurrency N` (or `-j N`, by default the CPUs' count up to 4) renders N chunks of frames, each on a thread with its own virtual clock, stage and encoder. The frames are those of one render, and an encoded video's chunks are joined without encoding them again. The showcase with motion blur renders about 2.6 times as fast with four chunks.
