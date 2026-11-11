---
section: fixed
---
- **A plugin's sandboxed compute passes take turns** (zgx-3d, `compute` feature): where a frame had room for only some of an owner's passes, its first pass ran every frame while the others shared what was left. The least recently run now go first. Sandboxed passes run wherever the device has timestamps, which apps now ask for before `Application::run`; elsewhere they're refused, as before.
