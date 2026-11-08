---
section: fixed
---
- **Light text below 2x keeps its stems one pixel wide** (zgx-gpu): the firming that gives light text on a dark ground its weight below 2x lifted every partly covered pixel, including the faint column a stem spills into its neighbour, so on a 96 dpi screen a one-pixel stem read as a soft two-pixel one. It now firms a stroke's own edges only (coverage from 0.35, fully from 0.7) and leaves the spill as it is. At 2x and over, and for dark text, nothing changes.
