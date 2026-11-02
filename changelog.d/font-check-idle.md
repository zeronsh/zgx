---
section: performance
---
- **The system font cache is checked as the app first goes idle, with no thread** (zgx-gpu): the check that a cached font file hasn't changed in place ran on a thread of its own two seconds after launch, and a process that has ever had a second thread locks glibc's allocator on every allocation. It now runs on the renderer's first trim (as the host's frames stop, no wake of its own), or after 600 frames for an app that never goes idle. On Linux, the headless lab stream no longer pays the locked allocator (about 12 M instructions).
