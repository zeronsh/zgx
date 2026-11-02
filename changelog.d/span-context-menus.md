---
section: added
---
- **Spans take context menus** (zgx, zgx-script): `ui.span { context_menu = … }` (`TextSpan::context_menu`) gives a run a menu of its own, on a link in place of Open Link and Copy Link; a secondary click inside the text selected shows the selection's menu (Copy, Select All), over a plain link's too, and Shift+F10 or the Menu key opens a focused link's. A `ui.rich` paragraph's `context_menu` is told the run it was asked on (`e.span`: `{ index, href, text }`; `View::context_menu_with_span` and `ContextMenuSpan` in Rust), and asked first on its links. A span's `on_click` returning `true` keeps its `href` from opening (`TextSpan::on_click_handled`). `zgx test` gains `app:right_click(target, { x, y }?)` and `app:opened()`.
