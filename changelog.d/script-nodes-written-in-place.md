---
section: performance
---
- **A script's node is written into its place** (zgx-script): each `ui.*` call's node (hundreds of bytes) is written once into the allocation that holds it, where `Rc::new` moved it twice. The mount benchmark does 0.26% fewer instructions.
