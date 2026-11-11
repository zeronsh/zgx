---
section: added
---
- **`GpuRenderer::finish` submits the last frame and waits for the GPU** (zgx-gpu): `render` leaves a frame for the next frame, a present or a readback to submit, so an offscreen bench or test timing frames without reading them back timed frames never submitted. Not for an app's frame loop, where presenting submits.
