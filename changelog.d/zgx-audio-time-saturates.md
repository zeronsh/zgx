---
section: fixed
---
- **A time past the clock's end is its end** (zgx-audio): `Time + frames` saturates, so `audio.now() + audio.seconds(f64::INFINITY)` is never, not a panic in a debug build and a time in the past in a release one. A device whose clock reads near its end no longer overflows the time a sound is heard.
