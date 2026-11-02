---
section: performance
---
- **A reflow with a screen reader running sends only the nodes near the window** (zgx-desktop): a node more than a window away, still on the side of it it was last sent on and otherwise unchanged, keeps the box it was sent with until it comes near; above stays above and below below, so a reader's order and the way it scrolls are the box's. Resizing a 3,000-row list with an assistive client attached: 3,032 → 178 nodes an update, AccessKit and its adapter 3.7 → 0.25 ms, zgx's projection 4.5 → 3.3 ms.
