---
section: added
---
- **A video can draw its sound** (`zgx-video`): `video.audio_data(src)` gives a sound file's spectrum as bars (`song:bars { count }`), its loudness (`song:level()`) and its waveform (`song:waveform { count, span }`) at each frame, analysed in Rust and the same whichever frame came before.
