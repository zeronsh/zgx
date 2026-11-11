---
section: changed
---
- **Nothing of the engine waits at idle** (zgx-audio). A stream paused (`Stopping::Pause`) is closed after `Config::close_after` (10 s), since a paused WASAPI stream still wakes the process. Once a stream is closed, the engine's `zgx-audio` host thread hands its device back and ends, and the next sound starts another.
