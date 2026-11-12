---
section: fixed
---
- **A one-line ellipsis cuts inside the word that overflows, as CSS's does** (zgx-gpu, text): with `line_clamp` of 1 and an ellipsis, a line whose next word did not fit ended at the previous word, so `aa bbbb…` showed `aa…`. The ellipsis now cuts inside that word, where the line overflows.
