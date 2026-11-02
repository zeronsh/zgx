---
section: performance
---
- **Accessibility updates hash with FxHash** (zgx-desktop): the projection's maps, keyed by node ids zgx hands out, hashed with SipHash, ~17% of each update. Resizing a 3,000-row list with an assistive client attached: 6.4 → 4.2 ms an update in zgx's projection (median of three runs each). Nothing changes with no assistive technology running.
