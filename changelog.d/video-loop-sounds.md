---
section: fixed
---
- **A sound inside `video.loop` plays on every pass** (zgx-video): each pass placed it at the loop's start, so it sounded once; it's now placed at the start of the pass it mounts in, as its pictures already are. A sound under a `video.freeze` keeps its place.
