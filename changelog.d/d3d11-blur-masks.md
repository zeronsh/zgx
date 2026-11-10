---
section: added
---
- **Direct3D 11 draws content blurs and masks** (zgx-gpu, the experimental `d3d11` feature under `ZGX_D3D11=1`): a frame holding one drew nothing, failing with an error naming it; their passes now draw into the effect atlas with the same shader as Direct3D 12, byte for byte against it on WARP.
