---
section: performance
---
- **zgx's inlining no longer stops at rustc's default depth** (zgx): the crate's recursion limit, which also bounds how deep rustc's MIR inliner goes, is 256, so 207 functions that reached the default 128 (mount builders, accessibility, selection) inline what they call as far as the rest do, and a hot function can't lose its inlining silently to a change elsewhere.
