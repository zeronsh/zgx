---
section: added
---
- **Binaural sound without a measured set** (zgx-audio, `hrtf` feature): `Binaural::spherical()` hears through Brown and Duda's spherical head (`HrtfSet::spherical_head`). It is made in code at the engine's rate: each ear's delay from the head's centre, which between the ears is Woodworth's formula, and the head's shadow on each ear as a one-pole, one-zero filter. It places sounds left and right and dulls what's behind an ear. It has no outer ears, so elevation and front from back are weak; a measured set given to `Binaural::new` replaces it.
