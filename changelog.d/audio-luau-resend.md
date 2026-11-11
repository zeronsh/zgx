---
section: fixed
---
- **A reload stops the old script's sounds even when the mixer's queue is full** (zgx-script, `audio`): a voice's stop or an effect's removal the queue refused was dropped, leaving the sound playing or the effect on its bus. It's now sent again until the queue takes it, and an effect stays counted until then.
