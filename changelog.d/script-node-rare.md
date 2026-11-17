---
section: performance
---
- **A Luau node takes about half the memory** (zgx-script): the fields few nodes set (`layout`, `mask`, `theme`, `tooltip`, `disabled`, `pointer_events` and the rest) and the drag and drop handlers are each boxed, made only for the nodes that set them. A Luau table of 10,000 rows in the browser peaks at 43.6 MB of wasm memory, down from 47.8, and a second create of the rows holds 34.4 MB live rather than 38.1.
