---
section: added
---
- **A Luau app's `zgx.json` can ask for GPU timestamps and compressed textures** (zgx-run): `"gpu": { "view_features": true }` makes zgx-run ask before it starts the device, so a `ui.scene3d` scene's device has each feature its GPU has. Without the key, the device request is exactly what it was.
