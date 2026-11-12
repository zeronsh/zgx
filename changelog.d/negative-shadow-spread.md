---
section: fixed
---
- **A negative box-shadow spread shrinks the shadow, as CSS's does** (zgx-gpu): spreads were clamped at zero, so a shadow like `0 10 20 -3` drew too large and too dark. The shadow's box and corner radius now shrink by the spread, down to a point.
