---
section: fixed
---
- **`ui.find_look`'s icons come from the host's files**: an icon named by path is read as images are, so an app run from a bundle (`ScriptHost::bundled`) loads it from the bundle, not the disk. A failed image's message names its script line in a bundled app too.
