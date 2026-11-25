---
section: added
---
- **A host reads the message catalog its scripts do** (zgx-script): `ScriptHost::i18n_catalog()` gives the catalog `ui.t` and `ui.locale` use (hidden from the docs for now), or `None` before one is loaded, so a host's own labels take the same messages, arguments and locale.
