---
section: added
---
- **zgx-audio streams music and reads Ogg Vorbis** (zgx-audio, feature `vorbis` for Ogg): `Audio::stream(source, play)` decodes as it plays, about half a second ahead, on one thread that sleeps when no stream is open. A stream loops without a seam, and when its decoding falls behind it fades out over 5 ms and back in rather than clicking (`Stats::dry`). `Audio::load` reads Ogg Vorbis as well as WAV. Offline renders wait for the decode thread, so they come out the same every time.
