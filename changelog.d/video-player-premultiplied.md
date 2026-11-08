---
section: fixed
---
- **A player's translucent picture shows as drawn** (zgx-video, zgx, zgx-gpu): the frames a `Player` reads back are premultiplied, and shown as images they were premultiplied again, darkening every translucent edge (half-transparent red over white showed (191, 127, 127), not (255, 127, 127)). `ImageData::new_premultiplied` (zgx) makes an image of pixels premultiplied already, which the renderer draws as they are, and `is_premultiplied` says which an image holds; the player's frames are made so.
