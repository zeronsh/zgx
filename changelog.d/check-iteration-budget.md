---
section: fixed
---
- **`zgx check` passes a props type made with `Merge` handed back to an element** (zgx-script): `Merge<ContainerProps, { … }>` given to `ui.column` failed as "too complex to typecheck" once an element's props grew past luau-lsp's iteration budget, so every new prop tipped some app over. `zgx check` gives the solver five times the default budget (`LuauTypeInferIterationLimit=100000`); set the same in your editor's `luau-lsp.fflags.override` (docs/scripting.md shows it).
