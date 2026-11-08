---
section: performance
---
- **A list growing at its end looks at its new rows alone** (zgx, zgx-script): a `ui.each` over a list that keeps its first items (a streamed reply's blocks, its last one growing) matches only the items after them to rows, and `keyed` mounts, removes and hashes only the keys after an unchanged prefix, where that is well under the whole list. Each chunk of a streamed Markdown reply had hashed every block's key again: the `md-stream` cell (400 sections) does 24.4% fewer instructions, with the same rows, order and reuse.
