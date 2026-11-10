---
section: added
---
- **zgx-audio, a crate for sound** (zgx-audio): a mixer that places every play, stop and parameter change on an exact frame. It has voices with gain, pan and pitch ramped a sample at a time, buses with gain, mute, solo and fades (linear, equal-power, exponential), a look-ahead limiter on master, WAV files at their own rate, and an offline renderer. No zgx crate depends on it, so apps that don't use sound pay nothing. Device output comes in a later release.
