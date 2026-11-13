---
section: added
---
- **`pressed`, `inert`, a reactive `aria_hidden` and `sr_only`** (zgx, zgx-script): `pressed` is a toggle button's `aria-pressed`, read as `checked` is on a button; `inert` takes a subtree out of the pointer's reach, the focus order and the accessibility tree while true (focus inside moves on as it goes inert); `aria_hidden` takes a state or function as well as a boolean; and `sr_only` is Tailwind's `sr-only`, a 1 by 1 clipped box painting nothing that stays in the accessibility tree for a live region's text (`View::sr_only`, `inert_when` and `accessible_hidden_when` in Rust). `invisible` text and a node at no opacity leave the tree, as `visibility: hidden` does, which is why `sr_only` keeps a thousandth of opacity; a test says so.
