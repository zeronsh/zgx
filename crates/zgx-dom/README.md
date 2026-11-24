# zgx-dom

zgx views as browser DOM, from WebAssembly: a Rust-composed `zgx::compose::View` mounts into real elements that the browser lays out, shapes, scrolls and reads to assistive technology, and zgx's fine-grained signals write only the nodes that read them. No virtual DOM, no diffing.

```rust,ignore
let runtime = zgx::reactive::Runtime::new();
let mounted = zgx_dom::mount_in_body(&runtime, view);
```

It builds for `wasm32-unknown-unknown` only (elsewhere the crate is empty). With the `luau` feature, Luau apps (`zgx-script`) run in it too, Luau built with wasi-sdk (`luau-env.sh`). GPU content (shaders, 3D, GPU views) doesn't run in it yet. [docs/dom.md](../../docs/dom.md) has the design, what maps to what, what doesn't, and the benchmark against Solid.
