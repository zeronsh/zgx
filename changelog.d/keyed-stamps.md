---
section: performance
---
- **`keyed` looks each key up once a pass** (zgx): each child's entry notes the pass that last showed its key and where, so one lookup finds a key's child, refuses a key met twice and marks what stays; no set of the pass's keys is built. A row of a keyed `ui.each` built again (a selection, an edited item) costs 1.2% fewer instructions in `script-update`'s `rows` and `rows-in-place` scenarios.
