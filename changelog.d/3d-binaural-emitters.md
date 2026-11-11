---
section: added
---
- **Binaural emitters** (zgx-3d, new `hrtf` feature). `Emitter3d::binaural(true)` hears an emitter through a head (HRTF, the spherical-head model) rather than panning it, so it sits round the listener, for headphones. Each such emitter takes one of the world's heads: a bus with a binaural on it, facing the emitter as it moves. A world makes up to eight heads as needed and keeps them for its life. A head let go is given to the next binaural emitter, and past eight an emitter is panned. A binaural emitter still sends to the reverb zones. The far ear hears a source beside the listener 13.6 dB quieter and 0.67 ms later.
