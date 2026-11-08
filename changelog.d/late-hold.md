---
section: performance
---
- **A scroll at 60 Hz shows newer motion without costing the window server more** (zgx-desktop, macOS): a scroll's frame still starts as late as it makes its refresh, but hands over at the grid's point, about 1 ms before the refresh, as other steady frames do, instead of as soon as drawn, 6.5 ms before. On an M1 Max at 60 Hz the window server's time per frame is back where it was before late frames (and 4-11% under handing over at once), with motion 2-10 ms newer than that again.
