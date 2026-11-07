---
section: performance
---
- **A large script heap leaves less behind** (zgx-script): once a VM's heap passes 32 MB, its collector aims for 150% of the heap in use, where Luau's default lets it double; below 24 MB it goes back. The collector can't see what a script's objects hold in Rust, so a large heap left to double keeps long-gone rows and all they hold. A dashboard whose heap peaked at 115 MB peaks 18% lower for 2.2% more instructions a frame; smaller apps, and the mount and update benches, don't change.
