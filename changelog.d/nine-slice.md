---
section: added
---
- **Nine-slice images** (zgx, zgx-gpu, zgx-script): `Styled::image_slice(slice, widths)` (`Scene::set_image_slice`, `ImageSlice`), Luau `slice = { top, right, bottom, left }`: an image drawn in nine pieces (a nine-patch, CSS's `border-image` with `fill`). Its corners keep their size, its edges stretch along their side and its middle both ways, so one small image frames a button or a bubble at any size. Drawn as nine quads of the image's texture, each sampled half a texel inside its cut so pieces don't bleed into each other; widths past the box shrink together, as CSS's do. Images without a slice check one empty map.
