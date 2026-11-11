---
section: added
---
- **Streams keep to a memory budget** (zgx-audio): `Config::stream_memory` (8 MiB by default) caps the bytes all streams' buffers hold at once. Each stream holds its half-second ring and its window, 448 KiB for a stereo stream at 48 kHz. Past it, `Audio::stream` returns `Error::StreamMemory { asked, free }` rather than allocating. A stream's bytes come back when the mixer gives it back, and `Stats::stream_memory` says what's held.
