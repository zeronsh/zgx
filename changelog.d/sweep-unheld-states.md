---
section: fixed
---
- **A state keyed by a closed tab is dropped when nothing holds it** (zgx-script): `ui.state`, `ui.persist`-less module states and `ui.shared` keys named per tab (`chat:<id>`, `chat_live:<id>`, a popover's open flag) stayed in their store for the life of the window, a hundred and more entries each tab Anara opened and closed (415 to 595 over ten open/close cycles of the real app). The idle collection now drops, from the module stores and the shared store, each state no script holds that is still at its first value; it comes back the same when the key is declared again. A test opens and closes 80 keyed tabs and finds none left over.
