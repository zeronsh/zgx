---
section: fixed
---
- **An animated `ui.each` row follows its item** (zgx-script): with `animate`, `enter` or `exit`, a row whose item changed under its key kept the item it was built with, and a builder's third parameter was never passed. It now does as the plain `ui.each` does: a row whose item changes is built again (without its enter motion), or, if the builder takes a third parameter, gets its item as state and updates in place.
