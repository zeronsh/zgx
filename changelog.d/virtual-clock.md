---
section: added
---
- **Programmatic video** (`zgx-video`, new and optional; `zgx::clock`): a composition is an ordinary zgx interface, in Luau (`require("@video")`: `video.composition`, `video.frame`, `video.sequence`, `video.series`, `video.interpolate`, `video.spring`, transitions and `video.audio`), and `zgx video render <dir> out.mp4` draws it offscreen a frame at a time, no window and no browser, to H.264, HEVC, ProRes 4444 with alpha (by ffmpeg), a PNG for each frame or a GIF. `zgx::clock::VirtualClock` gives a thread its own time, which animations, `ui.after`, `ui.every`, shader clocks, GIFs, `ui.now`, `os.clock` and `math.random` follow there, so the same input makes the same frames. An app not using it reads one more flag where it read the clock.
