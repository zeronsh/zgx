---
section: added
---
- **Sound in 3D scenes from Luau** (zgx-script, `audio-spatial`): `ui.sound3d { sound = hum, loop = true, falloff = { ref = 2, max = 60 } }` in a `ui.scene3d` plays from where its object is, falling off with distance, panned, moved by Doppler and dulled by the air and the meshes between, and `ui.reverb_zone { size = { 20, 6, 20 }, preset = "hall" }` gives the camera a space's reverb inside it. zgx-run's `audio-spatial` (and `audio-all`) turns it on. Lint: `sound3d-no-max`.
