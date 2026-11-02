---
section: fixed
---
- **A test's timers keep its time** (zgx-script): under `zgx test` and `TestApp`, `ui.after` and `ui.every` fire as frames advance the test's time (1/60 s each), in order and each at its own time, never on the wall clock between two steps. A deferred `ui.after` in an app no longer lands mid-test on a slow machine. `ui.after(0, fn)` now runs `fn` on the next frame, in tests and apps; it asks for that one frame only, so an idle window goes back to sleep.
