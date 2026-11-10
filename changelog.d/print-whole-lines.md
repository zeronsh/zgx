---
section: fixed
---
- **A script's `print` reaches stderr a whole line at a time** (zgx-script): the line and its newline went out in two writes, and another thread's output in between (libwayland's `WAYLAND_DEBUG` trace) landed inside it, so a tool reading `LOCK true` read `LOCK true[3664360.153] wl_pointer@45.frame()`. So do `ZGX_PRESENT_LOG`'s and `ZGX_GPU_STATS`' lines.
