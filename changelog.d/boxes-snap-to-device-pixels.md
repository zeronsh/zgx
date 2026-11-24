---
section: changed
---
- **A box laid out between device pixels paints its edges on them** (zgx), as browsers snap layout boxes: a dialog centred at a quarter point draws its edges and a 1 pt border sharp on whole device pixels at 1x and 2x, where they blurred across two or three rows. Each edge rounds on its own, so boxes sharing an edge still share it, and a box paints exactly the device pixels whose centres lie in its layout box, so a press on any of them hits it. Text keeps its own snapping; what a translation moves between pixels (an animation in flight) and what a transform, a zoom or a tilt holds keep their places between pixels. Only with a pixel scale set (`Scene::set_pixel_scale`, as a window sets it).
