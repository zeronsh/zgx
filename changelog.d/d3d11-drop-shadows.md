---
section: added
---
- **Direct3D 11 draws drop shadows** (zgx-gpu, the experimental `d3d11` feature under `ZGX_D3D11=1`): they were left out of the frame; they now draw through the same effect passes as content blurs, byte for byte against Direct3D 12 on WARP.
