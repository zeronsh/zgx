---
section: added
---
- **A textarea's atoms can show as mentions** (zgx, zgx-script): `atom_style` (and an atom's own `style`) take a text colour, a weight, an underline in its own colour and offset at the font's thickness, and a leading icon in room before the atom's first word, which wraps with it (Luau internal, subject to change; `View::atom_style`, `EditorHandle::set_atom_style`, `Atom::style`, hidden, in Rust). A textarea without styled atoms draws as it did.
