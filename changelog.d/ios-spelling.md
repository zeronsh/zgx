---
section: added
---
- **`spellcheck` works on iOS, as UITextField's does** (zgx, zgx-desktop, zgx-script): `UITextChecker` finds the words; text already there isn't marked, a word gets marked when a typed delimiter ends it, with UIKit's round dots in the system red. The kit's composer turns it on by default; a plain `ui.input` still has to ask, as everywhere.
