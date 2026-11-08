---
section: performance
---
- **A render no longer plays its script through once more for its sound** (zgx-video): what its own stage hears placed as it draws is what the sound is made of, muxed in once the frames are encoded, where a pass of its own played every frame of the script again first (about 3.6 G instructions and a quarter of a second for the showcase, and in chunked renders before any chunk began). The streams are byte-identical; a single-file render's MP4 differs only in its `btrt` box's average bitrate, now the stream's own rather than 0. A file joined from chunks, or muxed, no longer loses its last frames to a sound that ends a moment before them.
