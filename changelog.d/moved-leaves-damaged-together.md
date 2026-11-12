---
section: performance
---
- **A node moved with nothing under it is damaged in one step** (zgx): `Scene::set_transform` works a childless node's old and new painted areas out together from one look at it, and adds them as one when they overlap along an axis (or marks them on the damage grid), where it walked the node's subtree twice. The damage is the same rectangles. Rows animating their order (the script-layout benchmark) do 13% fewer instructions, the anime benchmark 5%.
