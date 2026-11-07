---
section: added
---
- **Loops, freezes and volume curves in a video** (`zgx-video`): `video.loop { duration, times }` plays its children again each pass, their time and what starts as they mount starting again; `video.freeze { frame }` holds them at one frame; `video.audio { volume = function(frame) … end }` shapes a sound's loudness over its frames.
