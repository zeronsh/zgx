---
section: added
---
- **A reverb on buses** (zgx-audio): `zgx_audio::effects::Reverb::new(size, decay)`, a feedback delay network of eight lines fed back through a Householder matrix, each loop damped, with damping, pre-delay, wet and dry. Its sound falls 60 dB in the decay asked for, measured, and left and right stay apart; a bus with it runs on until its tail is 90 dB down, so the device isn't stopped under it. Size and decay can move, landing on the same samples at any block size. About 104 instructions a frame.
