---
section: fixed
---
- A property bound to `ui.spring` moves by the time that passed between its frames, not by the display's reported refresh interval: where a display reports a shorter interval than it draws at (an iPhone), springs ran up to three times slow. It steps no longer than a 120 Hz frame however slow the display, so 24 or 30 Hz keeps the same curve, and a window back after a while catches up a quarter second, not all of it. `TestApp::report_interval` and `skip_frames` give a test such a display.
