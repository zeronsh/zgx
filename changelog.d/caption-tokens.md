---
section: added
---
- **A theme can set the colours of Windows' caption buttons apart from the rest of the app** (zgx, zgx-desktop): `Titlebar::Inset`'s buttons read `caption.ink`, `caption.ink.hover`, `caption.ink.inactive`, `caption.hover` and `caption.pressed`, and Close reads `caption.close`, `caption.close.pressed`, `caption.close.ink` and `caption.close.ink.pressed`. Unset, each is what the buttons showed before: `text.secondary`, `text.primary` and `text.tertiary` over `hover` and `pressed`, and WinUI's red under a white glyph. An app that draws its title bar glyphs in full text colour, with no dimming in an inactive window, sets them in its theme. Zeron does this.
