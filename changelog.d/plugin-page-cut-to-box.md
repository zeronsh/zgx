---
section: fixed
---
- **A plugin's page is cut to its box** (zgx): `zgx::compose::confined` clipped at a box a hundred times its parent's width, so what a `ui.plugin_page` drew or opened past its right edge showed and took the pointer inside the `ui.sealed` region around it. It's the parent's size now. `View::top_layer`'s holder had the same mistake, without showing: its panel was held to the window.
