---
section: added
---
- **Sound in space, as numbers** (zgx-audio): `zgx_audio::spatial` gives:
  - distance fall-off (`Falloff::inverse`, `linear`, `exponential`, `custom`), cones, and air absorption as a cutoff for a `LowPass`;
  - Doppler from an emitter's and the listener's velocities, an equal-power pan, and vector-base amplitude panning for surround layouts.

  An `Emitter` sends its voice the gain, pan and pitch for where it is, as ramps to its next update (`update_at` places them on the clock, so a render is the same at any block size). Voices take a pan law (`Play::law(PanLaw::EqualPower)`) beside the default balance.
