---
section: added
---
- **Sends** (zgx-audio): a voice can be mixed into up to two other buses besides its own, each at a level of its own (`Play::send(&bus, level)`, `Voice::set_send(&bus, level, ramp)`). Its sound after its gain, pan and own low-pass goes there, scaled: to a bus with a reverb, a binaural effect, or anything of the app's. A send ramped to 0 is taken away. A voice with no sends costs what it did (7.00 instructions a voice a frame).
