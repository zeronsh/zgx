---
section: performance
---
- **A box laid out by taffy finds each child's measure without hashing** (zgx): the vendored taffy keeps a node's context in a dense map by slot, where it hashed the slot with SipHash every time it laid a child out. A split dragged between panes of wrapping text costs 0.7% fewer instructions, and the mail and sketch apps 0.1%.
