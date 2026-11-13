---
section: fixed
---
- **A component's states and closures are collected after it unmounts** (zgx-script): `ui.component(build)` returned a Rust closure holding a handle to `build`, a root the collector cannot see past, so a `build` that reached its own factory (a recursive component, or any table of components its states hang from) was never collected. In Anara each chat open left about 120 states and 300 closures. The factory is now a Luau closure over `build`; a test mounts and unmounts such a component 100 times and finds no states left after a full collection.
- **`zgx inspect gc` shows what Rust holds**: Lua values Rust holds handles to (`Lua::handles_alive`), effects alive and the values effects own, beside the nodes alive.
