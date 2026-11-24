---
section: added
---
- **A native module moves and inserts rows of a shared list by key** (zgx-module, zgx-script): `Shared::upsert_at(key, rows)` gives each row a `Place` (just before a key's row, at the end, or where it is), so a module that diffs a reordered list on its own thread posts only the rows that moved or were made, not the whole list. One walk of the list; a row placed before itself, before a key no one has, or rows placed before one another are refused with nothing written. The host's `Shared` has the same, and `zgx_script::Place`. [The guide](docs/modules.md) says how.
