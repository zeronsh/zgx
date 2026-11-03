---
section: fixed
---
- **A flex item its content held wide shrinks again when that content gets smaller** (zgx): in a row or column too small for its children, an item held at its content's least size (CSS's `min-width: auto`) now shrinks as soon as that size drops: when a child starts clipping, leaves the flow (`absolute`) or gets narrower. Before, the item kept its old width until something else laid out its parent.
