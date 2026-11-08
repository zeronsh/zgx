---
section: fixed
---
- **A clip shows the file's frame on screen at its time, wherever it starts** (zgx-video): ffmpeg's rate conversion took the file frame nearest each of the composition's frames, so at a rate that doesn't divide evenly (a 7 fps file in a 30 fps video) a frame could show before its time, and a clip started between two of the file's frames could begin on the next one. The decoder now seeks a second early, keeps the file's times, takes for each composition frame the last file frame to start at or before it, and starts at the frame asked for, as `docs/video.md` says and as the iOS decoder does.
