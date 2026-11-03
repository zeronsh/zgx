---
section: fixed
---
- **Text taking its colour from a token around it follows every theme switch** (zgx): a text inheriting `$text.primary` (or any colour token) from a row or button around it took the new theme's colour on the first switch and kept it after: the second switch's paint-only style run saw nothing of its own change and resolved the inherited size and family again, but not the colour. Sidebar rows and button labels kept the old scheme's text after switching themes twice. The inherited colour is resolved again with them.
