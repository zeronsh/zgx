---
section: fixed
---
- **`ui.spring_to` keeps time on the interface's clock** (zgx-script): it read the wall clock for how long had passed since it was called, so on a clock of its own (a test's, a video made offline) its `delay` and the curve followed real time instead: a loaded test runner started a delayed spring early. It now reads `zgx::clock::now()`, which is the wall clock in a window, as before.
