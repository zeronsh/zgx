---
section: added
---
- **An editor measures its text at any width** (zgx, zgx-script): `EditorHandle::text_metrics(width)` and a script element's `el:text_metrics(width)` give the widest hard line unwrapped, the height wrapped at `width`, the line height and a revision, for sizing a composer to its text without a hidden copy of it. They measure what the editor shows (styled spans that shape, an IME composition, a final empty line), not its placeholder or insets, and are nil for a password field. A reader in an effect or computed hears content and typography changes once laid out, not focus, selection, scroll or the editor's own size. The first query at a new width or revision lays out every line, apart from the editor's own view; repeating it is free.
