---
section: added
---
- **A composition plays inside any zgx app** (`zgx-video`): `zgx_video::Player` makes its frames on a thread of its own, as a render would, and `player.view()` shows them in a Rust view, or `player.expose(&host, name)` gives a Luau script a `host:<name>` image and actions to drive it. The studio is built on it.
