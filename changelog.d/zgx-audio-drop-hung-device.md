---
section: fixed
---
- **Dropping the engine doesn't hang on a device that won't close** (zgx-audio): it waits up to 2 s for the device's stream to close, then leaves a driver that hangs to finish on the audio host's own thread.
