---
section: added
---
- **`zgx inspect gc --retained` says who holds what, Luau or Rust** (zgx-script): the answer counts the matches a Luau holder alone reaches, against the rest, held only through the handles Rust keeps (the auxiliary thread's stack, where every `Function`, `Table` or userdata a Rust object holds sits). `--lua-only` shows the Luau holders' chains (a global, a module table, a cache) instead of the handle chains, which otherwise win the shortest path to everything Rust also holds.
