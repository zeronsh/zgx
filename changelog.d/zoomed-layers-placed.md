---
section: fixed
---
- **In a zoomed window on macOS, baked loops and opaque GPU views lie where their boxes are** (zgx-gpu, zgx-desktop): their Core Animation layers were placed in the zoomed window's points where the host layer's hold no zoom, so a ring with a looping `shaderTime` floated up and left of its row at 110%, further the further from the window's corner, and was too small. A host sets it with `GpuRenderer::set_zoom`. A window read back shows the caret there too.
