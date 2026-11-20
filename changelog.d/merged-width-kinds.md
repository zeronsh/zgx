---
section: fixed
---
- **A width merged over another of another kind replaces it** (zgx): `Styles::merge` kept a box's points (or percentage) under a size expression merged over them, and the points won, so a menu or context menu given `w = "max-content"` stayed at the panel's default 240. Now either kind replaces the other, as a later CSS declaration does, for heights too.
