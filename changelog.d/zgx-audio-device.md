---
section: added
---
- **zgx-audio plays to the system's output** (zgx-audio, feature `device`): `Audio::new` opens nothing until the first sound, then streams to the default output through cpal, and stops after a second of silence. It starts again with the next sound, or early with `Audio::prewarm()`. A lost stream starts again on the default output, keeping its voices. `Stats` reports starts, stops, underruns, when the last play is heard on the device's clock, and the loudest sample sent; `Audio::block_times()` reports how long each block took to mix. `Device` is public, for other backends.
