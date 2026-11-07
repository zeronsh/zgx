---
section: performance
---
- **A window on Windows arms its wait's timer once per deadline, not once per event** (this repository's winit, so apps from crates.io, which get upstream winit, don't have it): a turn of the event loop that waited for a deadline set Windows' high-resolution timer again every turn, though it was already set for that time. Under a burst of input, such as a 1000 Hz mouse, that was a kernel call per move for nothing.
