---
section: fixed
---
- A property bound to `ui.spring` steps by the time that passed between frames, not by the display's reported refresh interval: where a display reports a shorter interval than it draws at (an iPhone), springs ran up to three times slow, and a late frame no longer slows one either. `TestApp::report_interval` gives a test such a display.
