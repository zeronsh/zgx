---
section: performance
---
- **The paint walk does less per node** (zgx): nodes it reaches through the tree (children, as the paint and ink walks find them) are looked up without checking their generation again; a style whose fade bands are all zero is known not to fade before the loop over them; and a frame with one damage rect (a scroll's, most) tests each node against it once. What is drawn is the same.
