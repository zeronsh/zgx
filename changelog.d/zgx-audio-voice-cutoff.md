---
section: added
---
- **A voice's own low-pass** (zgx-audio): `Play::cutoff(hz)` and `Voice::set_cutoff(hz, ramp)` give each voice a 12 dB, flat low-pass before its gain and pan. Its cutoff is ramped as the voice's other parameters are, and its response is the `LowPass` effect's. It is what a wall or the air between a sound and its listener does, for as many voices as play, where a filter on a bus of their own would need a bus each. At 20 kHz (the default) it runs nothing; a voice without one costs 0.06 instructions a frame more than before.
