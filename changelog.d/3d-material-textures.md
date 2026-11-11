---
section: changed
---
- **A custom material reads six textures, sampled 16 times anisotropically** (zgx-3d, `shaders` feature): `material_sampler` is now a sampler of materials' own, repeating, trilinear and 16 times anisotropic (it was the maps' 8), so ground seen at grazing angles keeps its joints and markings sharp; and a shader may take six texture parameters (it was four). docs/3d.md says how materials' textures are mipmapped and filtered, and where `dpdx`, `dpdy` and `fwidth` are good.
