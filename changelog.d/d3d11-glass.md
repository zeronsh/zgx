---
section: added
---
- **Direct3D 11 draws layer shaders reading the backdrop (glass)** (zgx-gpu, the experimental `d3d11` feature under `ZGX_D3D11=1`): they drew their box opaque; they now draw over a copy of what lies behind them, as Direct3D 12 does, each region's draws done before the copy as Direct3D 12 splits its pass. Backdrop blurs still draw opaque.
