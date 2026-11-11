---
section: performance
---
- **A folded row holds less until it opens, and a list's estimate frees what it read at idle** (zgx-script): a `Value::folded` row (a long transcript's) kept room for what it would open to, and an estimate reading it left that room holding the opened table's allocation. That room is now made only once a script or a host's `field` opens the row; an estimate's reading leaves nothing behind, and what it opened goes at the next idle rather than a free a row in the pass. 15,000 folded rows hold 20.7 MB rather than 21.3 at rest, 22.0 rather than 24.5 under an estimate, whose first pass costs 168M instructions rather than 183M.
