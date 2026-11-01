---
section: fixed
---
- **Glass on Windows 10** (zgx-desktop): `Glass` now blurs the desktop behind a window on Windows 10 (1803 and later) through `SetWindowCompositionAttribute`'s acrylic policy, and the tint that comes with it, keeping Windows 11's system backdrop where it exists. Before, only Windows 11 had glass: on Windows 10 a window asked for it was transparent with nothing blurred behind it, which read as a window with odd opacity rather than a translucent one.