---
section: added
---
- **Luau apps in the browser** (`zgx-dom`'s `luau` feature): zgx-script runs a Luau app in the browser and zgx-dom mounts its view as DOM, its state bound to the nodes that show it. Luau's VM is built for wasm32 with wasi-sdk (`crates/zgx-dom/luau-env.sh`), and zgx-script no longer links the renderer there. Images, `ui.fetch`, sockets, plugins and hot reload don't run in the browser yet ([docs/dom.md](docs/dom.md#luau-in-the-browser)).
