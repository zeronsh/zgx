---
section: performance
---
- **A script showing a video on macOS draws its frames as they are** (zgx-script, zgx-video): `ui.image { src = "host:<name>" }` shows a frame the app sets with the new `ScriptHost::surface` from its own storage, and a player exposed to a script (`Player::expose`) sets its pixel buffers so, where it read every frame back to bytes and uploaded them again. Its bytes are read only if something asks (`bg_image`, `dither`, `characters`, or the script). A player replaced under its name now stops, where it played on unseen.
