---
section: performance
---
- **A 3D world redraws only what changed** (zgx-3d): a frame where only the camera, a light other than the sun, the background or the exposure changed draws the meshes as the last frame did and uploads only the frame's 800 bytes of uniforms; one where some meshes moved makes and uploads only their instances (each object is stamped when what its instance reads changes, its children with it), in the order the last frame drew them, without sorting again. A still world of 100,000 meshes turns its camera for 105 K instructions a frame where it took 191 M.
