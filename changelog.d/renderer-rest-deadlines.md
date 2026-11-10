---
section: fixed
---

A window's CPU-only idle trims no longer postpone the renderer's queued memory and grown glyph atlas being released. Their existing rest delays still restart when a frame presents.
