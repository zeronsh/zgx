---
section: performance
---
- **A streamed column's layout costs the same however long the reply** (zgx): a column resuming at its end tells its kept children's places stand from the child at its end's place, not by comparing every child's id, and a reorder forgets what it kept. Debug builds still check what it keeps against every child, but not while phases are counted.
