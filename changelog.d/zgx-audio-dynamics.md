---
section: added
---
- **A compressor and ducking on buses** (zgx-audio): `zgx_audio::effects::Compressor::new(threshold, ratio)`, with a soft knee, attack and release (smoothed in decibels, the left and right reduced together) and makeup; and `Bus::duck(&dialogue, Duck::new(-9., 0.05, 0.4))`, which lowers a bus while another sounds, keyed on that bus's level over the block before. Its static curve holds within 0.1 dB of the ratio and knee asked for; its own logarithm and power cost a fraction of a library's and give the same samples everywhere.
