---
section: added
---
- **Binaural sound for headphones** (zgx-audio, `hrtf` feature): `effects::Binaural` hears a bus from a direction (azimuth and elevation, which can move) through an `HrtfSet`: the app's own head-related impulse responses, a left and right pair for each direction, at any rate (resampled to the output's, keeping their gain). The convolution is uniformly partitioned, so the same samples come out at any block size, a block (128 frames) late. A change of direction crossfades from the old pair to the new over a block. No response data ships with zgx: the built-in default is a spherical head made in code, and an app brings any measured set it uses.
