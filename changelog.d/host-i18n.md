---
section: added
---
- **`ScriptHost::i18n(dir, source)` loads a catalog with no script** (zgx-script): a host loads the locale files in a folder of its choosing before any script runs, and `ui.t` and `ui.locale` work in the app's scripts and its plugins' with no `ui.i18n` call. A hot-reloading host now watches every catalog's locale files, so an edit to one shows in place without reloading the scripts, wherever the folder is. A host that preloaded its catalog through a one-line plugin no longer needs that plugin, which made every hot reload a full one.
