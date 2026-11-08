---
section: fixed
---
- **A heavy scroll keeps the display's full rate on macOS** (zgx-desktop, zgx-gpu): while a scroll is live, a frame ready after its refresh's point is handed over at once, and no frame is held a refresh more to keep the last one's cadence. Frames needing about 5 ms or more at 120 Hz used to drop to every other refresh for the rest of the gesture. A live scroll's frames also draw into drawables that Metal presents once the GPU is done, so the main thread no longer waits for each frame's GPU work (`ZGX_SCROLL_DRAWABLES=0`: tiles, as before).
