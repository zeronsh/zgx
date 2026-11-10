---
section: fixed
---
- **Uncovered shader loops can play from baked frames** (zgx): a later container whose children paint elsewhere no longer blocks a loop merely because their combined ink bounds cross it. Overlapping paint still blocks it, and transformed or filtered subtrees keep their conservative bounds.
