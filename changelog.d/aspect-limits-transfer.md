---
section: fixed
---
- **A definite width keeps its width under a limit on its height** (zgx): with an aspect ratio, a `max_h` or `min_h` passes to the width only when the width is unset, as CSS transfers one. `w_full, aspect = 0.75, max_h = 320` in a 400 pt column is 400x320, not 240x320.
