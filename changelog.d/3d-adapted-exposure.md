---
section: added
---
- **A world says the exposure automatic exposure has adapted to** (zgx-3d, `post` feature): `World::adapted_exposure()` gives the stops the scene is exposed by, `set_exposure`'s compensation included, as last read back from the GPU. It is `None` before the first readback and again from when the exposure starts over until the next (turned on, settings changed, the renderer made again, after a pause), with automatic exposure off, and on devices without compute.
