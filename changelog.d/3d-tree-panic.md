---
section: fixed
---
- **A panicking occlusion tree strands nothing** (zgx-3d, `audio` feature). The tree thread catches a panic while making one tree and goes on to the next; that tree's mesh stays tested triangle by triangle. A tree whose thread went before making it is queued again. The tree thread, once made, stays for the life of the process, waiting without wakeups.
