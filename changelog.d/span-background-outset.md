---
section: added
---
- **A span's background can reach past its text without taking room** (zgx, zgx-script, zgx-gpu): `TextSpan::outset(x)` and a script span's `outset = x` widen its background `x` points on each side, as CSS's negative inline margin under the same padding does, and the text stays where unmarked text would be: a search match's mark.
