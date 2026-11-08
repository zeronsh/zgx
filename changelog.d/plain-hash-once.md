---
section: performance
---
- **A text the cache doesn't hold is hashed once** (zgx-gpu): measuring plain text hashed it to look it up, then again to file it. The second time it reuses the first, and heavy-both's flush is 0.4% shorter.
