---
section: fixed
---
- **A node tooltip's card is its popup's panel** (zgx-desktop, macOS): a popup window took the panel to be what its root holds, which for a node tooltip is its clear layer. A card with its own shadow then got the window server's shadow too, outlining the whole reach of the card's (a dark rim round empty space below it), and a translucent card got no material behind it. Through boxes that paint nothing and hold one thing, the card is now the panel: its own shadow alone, and AppKit's material under its translucent fill, cut to its corners, as a menu's.
