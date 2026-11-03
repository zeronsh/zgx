---
section: fixed
---
- **A token radius or border takes its single corners or sides, as a number does** (zgx, zgx-script): `rounded = "$app.r", rounded_tl = 0, rounded_tr = 0` painted all four corners at the token's radius, and `border = "$w", border_sides = { … }` every side at its width: the token resolved after the refinements and wrote over them. The token now rounds only the corners no single corner sets (and follows theme changes there), `rounded_corners` and `border_edges` replace a token radius or width as they replace a number, and padding and margin, which already did, are pinned by tests.
