---
section: added
---
- **A host can have an editor check its spelling again** (zgx): `EditorHandle::recheck_spelling()` forgets what a spellchecked editor (`View::spellcheck`) had checked and asks the platform about its whole text again, so the underline under a word the host just learned or ignored through the platform (`Platform::learn_spelling`, `ignore_spelling`) goes, and an edit elsewhere does not bring it back. It does nothing while the editor isn't spellchecked. zgx's own menu is unchanged.
