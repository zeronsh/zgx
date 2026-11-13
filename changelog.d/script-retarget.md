---
section: added
---
- **A host can swap its scripts in place, and try them first** (zgx-script, hidden for now): `ScriptHost::from_files` runs scripts held in memory (`MemoryFiles`), as a bundle's, and freed with the last host holding them; `retarget` loads another set in place of the app's, its interface built again with its state kept, and if that load fails nothing changes; `validate` loads a set in a host and runtime of its own, touching nothing of the app's. Both say whether a load failed or ran past its time budget (`ReloadError`). A retarget reads the app's string catalog (`ui.i18n`) again from the new files; `MemoryFiles::new_at` keeps the root the app's first files had.
