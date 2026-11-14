---
section: fixed
---
- **A mouse wheel's scroll shows a new frame on every refresh** (zgx-desktop, macOS): a frame applying a wheel's notch kept the cadence of the frame before it, as an animation's steady frames do, so once one frame showed a refresh late the frames after it, each finished in a couple of milliseconds, waited a refresh more (11-16 ms) until eight in a row came 1.5 ms early. A notch's frame answers input and keeps no later cadence now; ready past its refresh's point it still waits for the next, so each refresh takes one frame. Scrolling a deep transcript under glass by a wheel tape: 104-117 frames a second with 3-15 missed refreshes became 119.6-120 with at most 0.4.
