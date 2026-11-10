---
section: added
---
- **Filters and an EQ band on buses** (zgx-audio): `zgx_audio::effects::{LowPass, HighPass, Peaking}`, state-variable (Andrew Simper's, solved by the trapezoidal rule), 12 or 24 dB an octave with a resonance. Their response is the cookbook biquads': it matches miniaudio's filters to 101–121 dB under the signal, at 27.7 to 42.8 instructions a frame against miniaudio's 46.5 to 133.6. A cutoff, resonance or gain can move a frame at a time (`EffectHandle::set(LowPass::CUTOFF, hz, ramp)`) with no zipper and the same samples at any block size, for about 90 instructions a frame while it moves.
