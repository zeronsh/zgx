---
section: performance
---
- **A parent's new percentage bases read its children's cached bits** (zgx): when a taffy-laid-out parent's bases change, it reads each child's percentage flags from the bits cached with what the child reads (`OWN_PERCENT_*`) instead of working them out from its styles each time. x86: widgets −0.08%, mount −0.06%, micro-state and the scroll scenes −0.05%, app-chat −0.04%, every cell ≤0. arm64: mount −178K instructions, micro-vscroll-variable −78K, heavy-stream −40K, app-chat −29K, app-mail −20K (#1276).
