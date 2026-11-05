---
section: fixed
---
- **A long list a script has let go of leaves as the window goes idle** (zgx-script): each list and map shown to a script stays converted, and alive in Rust and in the script's heap, until nothing else holds it and the cache next doubles in weight. After a long list was replaced by a short one that might be never, so a long list switched away from stayed in memory. Idle now lets go of what only the cache holds, as it collects the script's garbage.
