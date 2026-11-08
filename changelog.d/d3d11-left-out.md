---
section: changed
---
- **Direct3D 11 leaves out what it can't draw yet where that can't mislead** (zgx-gpu, the experimental `d3d11` feature under `ZGX_D3D11=1`): drop shadows are left out, images a GPU view draws are left empty, and a backdrop blur or a layer shader reading the backdrop (glass) draws its box opaque in its own colour, as Reduce transparency draws glass; each kind is named once on stderr. A frame holding one of these failed whole before. Content blur, masks, blend modes and image effects still fail the frame, naming what's missing.
