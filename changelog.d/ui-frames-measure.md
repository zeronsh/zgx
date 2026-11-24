---
section: added
---
- **`ui.frames.measure(fn)` and `ui.frames.begin()` tell how smoothly an action's frames came** (zgx-script, zgx-dom): for analytics, counted as a web page's `requestAnimationFrame` measures count them, so the desktop's and the browser's numbers compare: `duration_ms` (the second frame), `blocked_ms` (gaps over 50 ms, whole), `settled_ms`, `frames`, `long_frames`, and each frame's time (`times`) to work out more. `begin` records a gesture until `finish`, 30 s at most. Nothing runs while no measure does.
