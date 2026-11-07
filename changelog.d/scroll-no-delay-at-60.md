---
section: performance
---
- **A trackpad's scroll follows the fingers more closely at 60 Hz, and slow scrolling no longer steps at 30 Hz** (zgx-desktop): at a refresh of 12 ms or longer, a scroll stands 8 ms behind the motion that has come, not a whole event interval, while events keep up. Events about 30 a second, on any display, are bridged rather than running out between them and jumping.
