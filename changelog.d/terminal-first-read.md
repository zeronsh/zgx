---
section: fixed
---
- **A terminal opened while its window runs no longer panics on its first output** (zgx-terminal). Output read before the view's first layout built the rows while the slots they take theirs from were still borrowed. The panic was contained, but it ended the tasks of that tick with it, an app's among them.
