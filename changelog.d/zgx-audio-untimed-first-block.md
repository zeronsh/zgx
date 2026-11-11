---
section: fixed
---
- **A play that starts the stream is timed on WASAPI** (zgx-audio). WASAPI fills its first block before its clock runs, and ALSA's clock can read 0 before its first interrupt, so `Stats::play_asked` and `play_heard` read 0 for a play that started a cold stream. A block asked for at 0 now counts as untimed, and a play in an untimed block is timed by the first timed block after it.
