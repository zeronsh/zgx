---
section: added
---
- **A text area's atoms on the DOM** (zgx, zgx-dom): a text area with atoms (`View::atoms_when`, `on_atoms`, `atom_wash`, `atom_style`) is a contenteditable over its model, as the native one is: each edit (typing, deleting, a paste as plain text, undo and redo) is the model's, the text shown again from it (only what changed), the caret put back from it; an input method's text is the model's once committed; its atoms show as chips (their wash, colour, weight and underline). A text area without atoms is the browser's own field, as before. Copy, accessibility and the pointer over atoms follow.
