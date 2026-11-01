---
section: fixed
---
- **Apps on different zgx versions keep a system font cache each** (zgx-gpu, macOS and Windows). The cache was one file, so two apps whose zgx read different formats each rewrote what the other had written, and both loaded every font at every launch. The file is now named for its format (`system-fonts-v3`). Another format's file, or one left half-written, is removed once it has gone unchanged for 30 days; an app keeps its own file marked as in use.
