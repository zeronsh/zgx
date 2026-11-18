---
section: fixed
---
- **`gap_x`, `gap_y`, `min_w`, `max_w`, `min_h` and `max_h` take a theme token** (`"$space.1"`) as `w`, `h` and `gap` do; they ignored one before. And `gap_x` or `gap_y` refine `gap` whichever form each is given in (`gap = "$space.1", gap_y = 0` spaces lines 0 apart, as CSS's `row-gap` over `gap`). A layout that gave one of these a token now gets its size: a row with `min_h = "$space.12"`, say, is that tall where it was its content's height.
