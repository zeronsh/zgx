---
section: performance
---
- **Typing and clicking fast keep the display's full rate** (zgx-desktop, macOS): a frame answering a press, a key or text draws at once and shows as soon as it's ready, as a scroll's does. Frames ready near their refresh point used to keep the every-other-refresh cadence of the frames before them, so a dashboard re-rendering on each input showed 60 frames a second on a 120 Hz display; it now shows about 117. Frames an app's own timers make keep that cadence, which keeps a stream from alternating between one refresh and the next.
