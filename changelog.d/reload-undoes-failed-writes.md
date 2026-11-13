---
section: changed
---
- **A reload that fails undoes what its scripts wrote** (zgx-script): a script that set `ui.state`, `ui.persist` or `ui.shared` values as it loaded, then failed, left those writes in place, and the keys it made, though the interface it built never showed. Now each value is set back to what it held, the keys it made go, and the overrides plugins gave stay as they were: a failed reload leaves the app as it was. A host's first load, with nothing before it to go back to, records nothing and is as before.
