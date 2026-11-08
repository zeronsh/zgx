---
section: performance
---
- **A frame shown as one of the renderer's own surfaces no longer waits for the GPU to finish it** (zgx-gpu, macOS): it shows once its work is scheduled, and the next frame waits for it only where it writes its ring buffers again. The main thread waited out the window server's and other apps' GPU work at every frame: `present` went from about 5 ms (13 ms at p90) to 0.25 ms on a 6K display. `ZGX_PRESENT_COMPLETED=1` waits as before.
