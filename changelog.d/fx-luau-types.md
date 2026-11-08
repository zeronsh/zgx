---
section: fixed
---
- **zgx-fx's Luau module type-checks in an app's scripts** (`luau/fx.luau`): its effects took their options as required, so `fx.glow { preset = "neon" }` and a bare `fx.ripple()` were type errors under luau-lsp, and `fx.with`'s result didn't fit the element it went to. Its options are now typed optional and its effects and `fx.with` typed for what they are. `ui.glass { … }` also takes a plain `h = 84` again: its props merge over a container's, as a scroll's do, where an intersection rejected plain values. The gallery's one-shots fire where they're pressed (it read a click's position, which a click doesn't carry), and CI type-checks the gallery with the other examples.
