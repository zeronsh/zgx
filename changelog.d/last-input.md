---
section: added
---
- **A window tells when it last had input and whether anything is over its content** (internal for now): `Ui::last_input` (a key, the pointer, the wheel, text; not its focus coming or going), for an app's idle gate, and `Ui::overlay_open` (a modal, dialog, menu or popover in the window, or a popup window). No clock is read for each pointer move.
