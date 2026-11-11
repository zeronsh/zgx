---
section: fixed
---
- **A plugin's sounds stay in its directory and count against its memory** (zgx-script, `audio`): `audio.sound` and `audio.music` in a plugin read only its own files (no `..`, absolute paths or links out of it), its streams count among its 8 voices, and its decoded sounds against its memory limit. Sounds decode on two threads that sleep while idle rather than a thread each, and the app's cache lets go of sounds no script holds. A gain, pan or pitch given `0 / 0` keeps its last value instead of keeping the window drawing every frame.
