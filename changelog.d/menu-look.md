---
section: added
---
- **`ui.menu_look` styles every menu the app doesn't style itself** (zgx-script): `ui.menu_look { panel = {…}, item = {…}, hover = {…}, focus = {…} }`, in plain values, lies under each menu's and item's own styles, so the menus zgx builds (a link's Open Link and Copy Link, an editor's, a selection's) look as the app's own. `zgx::compose::set_menu_look` from Rust.
