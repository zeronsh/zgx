---
section: added
---
- **zgx's own words read in the app's locale** (zgx, zgx-script, zgx-components): `zgx::i18n::label("Copy")` and `label_args("Page {page} of {total}", …)` give the app's translation when its catalog has that English key, else the English as written, with no missing-key issue. A script host's catalog is the app's (`I18n::install_as_app`, which `ScriptHost::i18n` and `ui.i18n` call); a code block's Copy and Copied and its "Code" label, the find bar's Find, Previous, Next and Close, a toast's Dismiss and a date picker's month buttons read through it and follow a locale switch.
