---
section: performance
---
- **A 3D world's post chain resolves its samples in its last pass where it can** (zgx-3d, `post` feature): with nothing before the last pass needing the scene (no bloom, app HDR pass, automatic exposure, temporal antialiasing or chromatic aberration), the multisampled colour is resolved, weighted by the tone curve as before, where it is mapped: no scene texture (16.6 MB at 1920 by 1080) and one full-size pass fewer a frame.
