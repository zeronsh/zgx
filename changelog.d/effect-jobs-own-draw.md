---
section: fixed
---
- **A layer drawn again for the budget no longer panics over its blurred layers** (zgx-gpu): a layer past the layer budget draws again, coarser, in the room left. The effect slots (blurs, masks, drop shadows) its first try planned for the layers inside it stayed queued, and the first of those layers drawn again (one waiting on a picture or a shader) took them while its own texture was out of the cache: `no entry found for key` in `encode_effects`, ending the frame or a video's render thread. The first try's slots are now given up and planned again with the second.
