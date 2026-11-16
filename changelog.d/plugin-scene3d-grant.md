---
section: changed
---
- **A plugin draws 3D only when granted `"scene3d"`** (zgx-script, `scene3d`): `ui.scene3d` and `ui.model` in a plugin are errors unless its host grants it `Capability::Scene3d` (`"scene3d"` in its `allow` in zgx.json). Before, any plugin of an app with the `scene3d` feature could draw on the GPU. To keep a plugin's scenes, add the grant.
