---
section: added
---
- **A large script heap can ask its collector for less** (zgx-script): `ScriptHost::large_heap_goal(percent)` has a VM whose heap passes 32 MB aim its collector at `percent` of the heap in use (Luau's own lets it double, 200), going back once it settles below 24 MB. The collector can't see what a script's objects hold in Rust, so a large heap left to double keeps long-gone rows and all they hold. It trades CPU for peak: a script churning 100 MB of rows peaks about 23% lower at 150 for about 9% more instructions, and rests where it would. Off by default. No call pays to look: the VM's allocator notices the heap crossing, and the goal changes as that call ends.
