---
section: added
---
- **A modifier pressed alone is heard: `Ui::modifiers()` and `ui.modifiers`** (zgx, zgx-desktop, zgx-script): the modifier keys held now, as a state that changes as one goes down or up, a modifier alone included (Command held to show ⌘1–9 hints, which no key event reports). Luau reads `ui.modifiers.shift`, `control`, `alt` and `meta`, each tracked. All are up as the window loses focus, so a ⌘-Tab away leaves none held. The state is made the first time something asks for it; until then the host writes nothing.
