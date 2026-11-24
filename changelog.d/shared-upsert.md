---
section: added
---
- **A native module patches a shared list by key** (zgx-module, zgx-script): `Shared::upsert(key, rows)` replaces rows by a string field where they are and adds the new ones at the end, and `Shared::remove(key, keys)` removes them, so a module can diff its model on its own thread and post only the rows that changed. One pass over the list whatever the batch, the rows not given kept as they were. The host's `Shared` has the same, as `upsert` and `remove_rows`. [The guide](docs/modules.md) says how.
