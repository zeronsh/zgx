---
section: performance
---
- **A display link's frames no longer re-arm the event loop's timer each frame** (zgx-desktop, macOS and Windows): the native frame source's watchdog stays where it is while ticks come, and moves on to 250 ms after the last tick only when it falls due, so the run loop's timer is set about four times a second instead of once a frame. A link that stops is still backed up 250 ms after its last tick.
