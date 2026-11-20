---
section: fixed
---
- **A menu with `w = "auto"` is as wide as its widest row** (zgx, zgx-script): a menu's panel (and a context menu's) holds its rows in a scroll viewport whose content width was set to the viewport's measured width, which starts at zero, so a panel sized by its rows stayed at its padding. The content is now the viewport's width where the viewport has one (rows wrap or truncate to a fixed or default width, as before) and its rows' own where it is sized by them: `context_menu_style = { w = "auto", min_w = 160, max_w = 320 }` sizes a context menu to its widest row between 160 and 320.
