---
section: fixed
---
- **A raster window marks its first frame as presented** (zgx-desktop): a window drawn on the CPU (`ZGX_RENDERER=raster`, or no GPU) shows each frame as it draws it, but only the GPU renderer's present marked the process's first. On macOS the menu bar, which waits for that first frame, then waited out its one-second limit in a raster window; it now comes with the first frame, as in a GPU window. `ZGX_SHADER_LOG=1`'s `zgx: first frame presented` line and `ZGX_PROFILE`'s mark come from either.
