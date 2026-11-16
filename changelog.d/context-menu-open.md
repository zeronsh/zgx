---
section: added
---
- **A node's context menu says when it shows** (zgx, zgx-script, zgx-dom; internal for now): `context_menu_open = state` on a node with a `context_menu` (Rust's hidden `View::context_menu_open`) is written true as its menu opens and false as it closes, by Escape, a pick, a click outside, Tab, the window losing focus, a newer menu or the node going; writing it false closes the menu. A PDF reader hides its floating selection popover while the page's menu is open, as the web's does. Nodes without it pay nothing.
