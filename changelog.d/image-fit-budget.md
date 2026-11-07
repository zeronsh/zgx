---
section: fixed
---
- **Pictures past the image budget always show, and raise no issue** (zgx-gpu): at the budget's end a picture is cut to at most 64 pixels on its long side rather than left out, and one drawn softer for the budget uploads once, not on every frame. Kept files decode no larger than their textures will be.
- **An image used as a mask takes the size it masks at** (zgx-gpu), not its file's. A kept file still decoding no longer fails the frame or shows the masked content unmasked.
- **Layers past their budget draw coarser with no issue raised either** (zgx-gpu): `zgx inspect memory` shows those layers and how many images are held softer, at no cost until it's read.
