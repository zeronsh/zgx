---
section: added
---
- **A textarea's atoms answer the pointer** (zgx, zgx-script): `on_atom_press(atom, at)` when one is pressed in place (true keeps the caret out of the chip) and `on_atom_hover(atom?, box?)` with the chip under the pointer and its box on that line, again when a scroll or a new layout moves it (Luau props internal, subject to change; `View::on_atom_press`, `View::on_atom_hover` in Rust). A textarea without them is unchanged.
