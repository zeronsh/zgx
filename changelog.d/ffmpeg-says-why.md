---
section: fixed
---
- **A render ffmpeg stops part way says why** (zgx-video): it said "ffmpeg stopped taking frames: Broken pipe", and now gives what ffmpeg said as it quit (a file it can't open, an encoder that refuses the size).
- **An AV1 render smaller than 64 pixels a side writes** (zgx-video): SVT-AV1 before 2.0, as Ubuntu 24.04 has, refuses such frames, so they go to libaom or rav1e first; rav1e is also used when it's the only AV1 encoder.
