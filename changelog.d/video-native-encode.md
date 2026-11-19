---
section: added
---
- **`zgx video render` writes H.264, HEVC and ProRes on macOS without ffmpeg** (zgx-video): where ffmpeg or its encoder is missing, macOS writes them itself (AVFoundation) instead of failing. The command now says which encoder writes each file and why. ffmpeg's encoders still write whenever they can, because they measured better at each quality (docs/video.md has the table). A render's sound, VP9, AV1 and video clips still need ffmpeg.
