---
section: fixed
---
- **A layout that never settles fails a test, without a panic** (zgx-script): `zgx test` and `TestApp` panicked when layout didn't settle in 64 passes (a list squeezed in a box easing shut, say), taking the whole run down. They now contain it as a window does: the last layout is kept, and the error naming what still moved is the app's (`app:error()`, `TestApp::error`, the inspector), so the test fails on it and the rest of the run goes on.
