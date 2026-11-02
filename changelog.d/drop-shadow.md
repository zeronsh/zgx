---
section: added
---
- **Drop shadows** (zgx, zgx-gpu, zgx-script): `Styled::drop_shadow(x, y, blur, color)` (`Scene::set_drop_shadow`, `DropShadow`), Luau `drop_shadow` written as `shadow` is: a shadow of what a node and its subtree paint, following their alpha rather than the box (CSS's `filter: drop-shadow`): an icon's, a cut-out's, a rounded card's. The node paints as a cached layer; its shadow is a second slot of the effect atlas, blurred by the same passes as `filter_blur` and tinted there, then composited under the layer: a shadow moved or faded composites again without repainting the layer, and one blurred anew re-runs the passes only. Nodes without one cost nothing. Not drawn by the Direct3D 11 proof, as no layer effect is.
