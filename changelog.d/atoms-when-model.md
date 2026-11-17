---
section: changed
---
- **`View::atoms_when` is given the text area's model** (zgx): its closure takes `&mut TextEditor` (its text, its atoms, a new atom's id) where it took `&EditorHandle`, which only a native window has; the DOM target keeps a text area's text in a model of its own, so one API serves both. No release had the old signature (it came with the unreleased atoms): a closure reading `handle.editor.borrow()` reads `editor` itself.
