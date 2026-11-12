---
section: added
---
- **Menus fade as a web page's do** (zgx): `ui.menu_look { fade = 100 }` (`MenuLook::fade`) fades a menu's panel in as it opens and out as it closes, as menu.tsx's does: opened from the keyboard it shows at once, and closed by choosing an item it goes at once; Escape or a click outside fades it out, the menu staying drawn (taking no input) until it has. Reduce motion shows and hides it at once.
