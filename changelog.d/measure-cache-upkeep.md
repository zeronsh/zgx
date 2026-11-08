---
section: performance
---
- **Layout keeps its measured sizes for less** (zgx): a result moved out of the few kept whole had its size checked against every size kept, one by one, to keep each constraint once, though it is nearly never there already; it is looked for first now, and taken out only when found. A node that hasn't changed looks for its new result's size the same way rather than going through them all. A dragged split's layout costs 2.8% fewer instructions; every result is the same.
