---
section: performance
---
- **A Luau list built in the browser keeps its garbage down** (zgx-script): there, the Rust bytes each node holds step Luau's collector, which saw only a pointer-sized userdata for each. The nodes a build lowers no longer pile up to a peak that wasm memory keeps for good. A Luau table of 10,000 rows peaks at 28 MB of wasm memory rather than 38, a second build holds 21 MB live rather than 31, and creating the rows takes 3% less time. The desktop is unchanged: it gives its peaks back, and its idle trim collects them.
