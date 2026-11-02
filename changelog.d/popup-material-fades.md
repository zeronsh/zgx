---
section: fixed
---
- **A tooltip's glass fades with its card** (zgx-desktop, macOS): since a node tooltip's translucent card gets AppKit's material under it, the material stayed at full strength while the card faded out (100 ms) and showed at once while it faded in: an empty frosted pane the card's size lingered as a hover card went. The material now takes the panel's opacity through what holds it (the tooltip's fade), and goes as it reaches 0.
