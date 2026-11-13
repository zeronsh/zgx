---
section: added
---
- **zgx views as browser DOM** (`zgx-dom`, new, wasm32): a Rust-composed view mounts into real DOM elements, laid out, shaped, scrolled and read to assistive technology by the browser, with each signal write touching only the nodes that read it: no virtual DOM. Updates reach JS once per frame as an operation buffer; static styles become shared classes, and hover, pressed and focus styles CSS rules; keyed rows move the fewest nodes and repeated rows are stamped from templates. Luau and GPU content don't run in it yet ([docs/dom.md](docs/dom.md)). zgx's clock reads the browser's on wasm32.
