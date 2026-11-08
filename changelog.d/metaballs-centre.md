---
section: fixed
---
- **`fx.metaballs` draws each blob whole at its centre** (zgx-fx): its antialiased edge came from the field's slope, which climbs without bound where a blob is centred, so a few pixels around each centre took in the background as dark specks (and each GPU drew them a little differently). The edge now comes from the field held under 1, the same at the edge and flat at the centre.
