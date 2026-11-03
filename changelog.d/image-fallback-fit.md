---
section: fixed
---
- **A `ui.image` with a `fallback` keeps its `fit`** (zgx-script): its picture shows inside a switch (the picture or its fallback), and the image's styles went on the switch, so `fit`, `object_position` and `image_rendering` reached no picture: it drew as `fill` (a 4:1 picture covering a 56 by 40 box drew 56 by 40, not 160 by 40). The picture takes them itself now.
