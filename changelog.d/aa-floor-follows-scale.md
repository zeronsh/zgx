---
section: changed
---
- **Above 2x, a rounded box's edges are one device pixel soft, not half a point**: its sides on whole pixels showed a grey column at 3x (212 of 255 beside full white), and its corners were softer than a `CALayer`'s. At 2x and below nothing changes.
