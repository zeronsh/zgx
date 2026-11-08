---
section: added
---
- **A window can draw every display refresh, or as fast as the GPU allows, and counts the frames it shows** (zgx-desktop): `WindowContext::frame_mode` is `FrameMode::OnChange` (the default: only when something changed), `EveryRefresh` (frames never stop, for an app's frame-rate setting) or `Unlimited` (unpaced, for measuring what a frame costs); both of the others repaint the whole window each frame. `WindowContext::presents` (`PresentStats`) counts presented frames and the longest wait between two, short of the window going still, for an app's frame counter.
