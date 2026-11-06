---
section: fixed
---
- **Trackpad scrolling keeps a steady speed in a busy window on macOS** (zgx-desktop): events a busy main thread read together were timed when read, so content sped up and slowed down by up to half, most visibly when scrolling fast. Motion is now paced from when each event happened (`-[NSEvent timestamp]`). Apps on upstream winit, and other platforms, time events when read, as before.
