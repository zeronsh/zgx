---
section: performance
---
- **A frame's body is steadier to compile** (zgx-gpu): an effect slot's lookup, inlined into the frame loop, was one more hash search for LLVM to fold with the loop's own, and any edit nearby re-rolled how that shared search compiled (±0.05% on frames that never use it). It's out of line now, and those frames run slightly fewer instructions.
