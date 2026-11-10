---
section: added
---
- **An echo on buses** (zgx-audio): `zgx_audio::effects::Delay::new(time, longest)`, with feedback, wet and dry, its line made in `prepare` as long as its longest time. Its time can move (`Delay::TIME`) and glides, reading between frames as tape does, with no click. Set as miniaudio's `ma_delay` is, it gives the same samples bit for bit at 12.5 instructions a frame against miniaudio's 41.6.
