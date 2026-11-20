---
section: added
---
- **Collision options on `Placement`** (zgx, zgx-script): `Placement::collision` (`Collision`; Luau `placement = { padding, shift, flip_align, more_room }`) sets how a popover or menu avoids the screen's edges: `padding` points kept off them, `shift` (on, as before) slides it along the edge to stay on screen, `flip_align` lines it up with the trigger's other edge where its own would reach past, and `more_room` takes the side with more room where neither fits. The defaults are the old behaviour.
