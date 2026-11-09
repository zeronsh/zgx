---
section: added
---
- **3D scenes and glTF models in a video** (`zgx-video`): `ui.scene3d`, `ui.mesh`, `ui.light` and `ui.model` draw in a composition (the `3d` feature, on by default), and a video's frame shows the models it draws from its first, read before it is drawn as its pictures are decoded (`zgx::image::whole_frames_on_this_thread`), so a render in chunks or a still has them in every frame.
