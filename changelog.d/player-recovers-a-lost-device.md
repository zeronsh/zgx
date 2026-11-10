---
section: fixed
---
- **A video player whose GPU device is lost draws again on a new one** (zgx-video): after a driver reset or a GPU removed, it stopped with "GPU device lost" until the app made it again; now it builds its renderer anew and goes on from the frame it was at.
