---
section: added
---
- **Direct3D 11 draws backdrop blurs** (zgx-gpu, the experimental `d3d11` feature under `ZGX_D3D11=1`): a backdrop blur, frosted glass among them, drew its box opaque in its own colour, and one over a box with no colour of its own failed the frame. They now draw as on Direct3D 12: blur.wgsl's passes into an atlas of their own, drawn back over the target, mixed by dual-source blending.
