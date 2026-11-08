---
section: fixed
---
- **Light text below 2x no longer thickens its faintest pixels** (zgx-gpu): the firming that gives light text on a dark ground its weight below 2x lifted every partly covered pixel, including the faint column a stem spills into its neighbour, so on a 96 dpi screen a one-pixel stem read as a soft two-pixel one. Pixels up to 0.35 coverage now keep the coverage they were drawn with, the lift grows to its full amount by 0.7, and fuller pixels firm as before. Firming scales with how light the text is, so black text and 2x and over are unchanged, and dark grey changes in proportion.
