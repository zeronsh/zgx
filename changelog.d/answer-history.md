---
section: fixed
---
- **An editor's undo and redo don't break after an app answers an undo** (zgx): a document loaded (`load_next`) as an answer (`answer_next`) to an undo kept the step that redid the text before it, and an answer as long as the text kept the steps before the undone one. Undoing or redoing them then broke the text, or panicked out of range or inside a character. A load now leaves nothing to undo or redo, and an answer to an undo nothing older to undo. Controlled fields that answer what is typed, as Zeron's do, reach these paths.
