---
section: fixed
---
- **Plays waiting for a voice cost the audio thread no more than a few** (zgx-audio): a play that finds every slot taken now waits in order behind the others, and only the first is tried again; before, every waiting play was tried every 64 frames, so 90,000 plays due at once took half a second a block. A stop or change for a voice that has ended no longer searches every command scheduled: 64 of them with 90,000 plays scheduled ahead took 50 ms a block.
