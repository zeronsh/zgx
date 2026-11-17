---
section: fixed
---
- **A control in a layer just built takes focus with a dialog open** (zgx): with a modal open, a button in a `ui.layer` built that frame (a `ui.presence`'s new content) focused from its `on_mount` was refused (`el:focus()` false), as the layer counted as one only once it had finished opening. It's live under the modal from its first mount, as it was once open.
