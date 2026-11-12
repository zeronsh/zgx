---
section: added
---
- **A markdown view's links take the app's menu** (zgx-script): `ui.markdown_view { link_menu = function(e) end }` builds a link's menu from `e.href` and `e.text` (and the point); nothing back leaves the link without one, so the press goes on to the paragraph's, in place of Open Link and Copy Link.
