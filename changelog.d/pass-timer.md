---
section: added
---
- **`GpuRenderer::debug_time_passes`** (macOS): a frame drawn with Metal directly is split at its backdrop blur's passes, each committed on its own, and `debug_take_pass_times` gives each part's GPU time: where a frosted panel's cost lies. Off by default.
