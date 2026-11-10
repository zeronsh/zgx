---
section: fixed
---
- **3D lights and cascades take only values they can draw** (zgx-3d): a light with an intensity or colour infinite or not a number, a position or direction that isn't one, or a range not over 0 shines nothing (it lit every pixel of its clusters with not-a-number before); intensities past 10^10 and colours past 10^4 are taken as those. `Cascades` set by hand are brought within their setters' bounds (a map of 0 texels lost the frame), and a camera at its own target no longer keeps a world with cascades drawing. `Cascades` and `PointLight` are `#[non_exhaustive]`: make them with their constructors. Clustered lights list at most the 65,536 nearest the camera and 255 a cluster; `World::lights_left_out` says how many a frame left out.
