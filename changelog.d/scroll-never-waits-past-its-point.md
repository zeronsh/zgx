---
section: fixed
---
- **A heavy scroll keeps the display's full rate on macOS** (zgx-desktop, zgx-gpu): while a scroll is live, a frame ready after its refresh's point is handed over at once, and no frame is held a refresh more to keep the last one's cadence. Frames needing about 5 ms or more at 120 Hz used to drop to every other refresh for the rest of the gesture. With `ZGX_SCROLL_DRAWABLES=1`, a live scroll's frames also draw into drawables that Metal presents once the GPU is done, so the main thread doesn't wait for each frame's GPU work (off by default: their latency is still being measured).
