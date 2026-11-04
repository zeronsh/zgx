---
section: performance
---
- **A `ui.list` doesn't build rows folded to nothing** (zgx, zgx-script): rows given almost no height all fit in the viewport, so a list with thousands folded away built every one of them. With `heights`, a row under a hundredth of a point isn't built, and a row of 0 takes no room, where it was laid out a point tall.
