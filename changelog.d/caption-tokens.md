---
section: added
---
- **A theme can set the colours of Windows' caption buttons apart from the rest of the app** (zgx, zgx-desktop): `Titlebar::Inset`'s buttons read `caption.text`, `caption.text.hover`, `caption.text.inactive`, `caption.hover` and `caption.pressed`, and Close reads `caption.close`, `caption.close.pressed`, `caption.close.text` and `caption.close.text.pressed`. Unset, each is what the buttons showed before: `text.secondary`, `text.primary` and `text.tertiary` over `hover` and `pressed`, and WinUI's red under a white glyph. An app that draws its title bar glyphs in full text colour, with no dimming in an inactive window, sets them in its theme. In a contrast theme the system's colours stand in for all but Close's.
