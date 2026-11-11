---
section: changed
---
- **Sound comes in parts** (zgx-script, zgx-run): `audio` is the core a UI app's clicks need (WAV, voices, buses, `sound =`), `audio-effects` adds effects on buses and `audio-vorbis` adds Ogg Vorbis; zgx-run's `audio-all` turns on every part. A build with `audio` alone no longer links the stock effects or the Vorbis decoder.
