---
section: performance
---
- **A picture nothing holds any more gives its texture back as soon as no node shows it** (zgx, zgx-gpu): one replaced by another and dropped, as a video's frames are, can't be drawn again, so it no longer takes room among the textures kept for pictures scrolled away. A playing or paused video player held nine past frames there, 34 MB at 1280 by 720. Pictures an app or a cache still holds keep their textures as before. `ImageData::holders` tells whether anything holds an image.
