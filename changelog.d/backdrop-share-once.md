---
section: fixed
---
- **Glass draws once a frame, after the glass reading where it lies** (zgx-gpu): when a layer shader reading the backdrop (`zgx_backdrop`) couldn't share the copy of the target an earlier one took, readers after it shared both copies and drew twice, the second time over themselves; and one could draw before a reader whose read covered it. On Metal and Direct3D 12.
