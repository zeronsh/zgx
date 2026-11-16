---
section: performance
---
- **A canvas's line keeps its font uncopied** (zgx): a line drawn again in the font it had is compared with the node's in place, and its font is copied (its family, features and fallbacks each a shared count) only when it changed, and compared once. A terminal's log streaming in takes 0.2% fewer instructions.
