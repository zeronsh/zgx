---
section: fixed
---
- **A test's timers keep its time** (zgx-script): under `zgx test` and `TestApp`, `ui.after` and `ui.every` fire as frames advance the test's time (1/60 s each), in order and each at its own time, never on the wall clock between two steps. A deferred `ui.after` in an app no longer lands mid-test on a slow machine. A wait (`TestApp::wait_until`) delivers a frame when one is asked for, else moves the test's time on by the time that passes. `ui.after(0, fn)` runs `fn` before the next frame is drawn, as it does in a window: in a test, before the frame that made it ends, or as the next one starts.
