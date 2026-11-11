---
section: fixed
---
- **A 3D world drawn on devices of two wgpu instances keeps each one's resources apart** (zgx-gpu, zgx-3d): devices from different wgpu instances compare equal, so particles' pools and pipelines, compute's data and the app's GPU buffers made on one device could be bound on the other, ending in a "does not exist" panic. Each renderer's device now has an identity no other device in the process shares, `ViewportFrame::device_id()`, and zgx-3d keys what it keeps per device by it. A painter drawn with a device of its own, outside a renderer, takes an id with `zgx_gpu::viewport::new_device_id()` where it makes the device and hands it with each frame (`ViewportFrame::set_device_id`). Particles drawn on two devices now step on each.
