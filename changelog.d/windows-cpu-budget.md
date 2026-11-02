---
section: fixed
---
- **A script on Windows isn't stopped short of its time budget** (zgx-script): Windows counts a thread's CPU time a scheduler tick (15.6 ms) at a time, so a call that used 25 ms could read as two ticks, 31.25 ms, and be stopped under a 30 ms budget it hadn't spent. A budget's CPU time now counts as spent only a tick beyond it there; a runaway script stops up to that much later.
