---
section: performance
---
- **3D shadows find their pipelines by one byte** (zgx-3d): a frame drawing shadows looked up each run's shadow pipeline by a tuple of flags through the standard library's SipHash; it now uses one byte under the variants' own hasher. `3d-mixed`, which draws shadows, falls 2.2% (257 K instructions a frame).
