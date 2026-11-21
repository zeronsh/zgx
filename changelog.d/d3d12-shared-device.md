---
section: fixed
---
- On Direct3D 12, every renderer in a process draws with one device, held while one is alive: renderers made and dropped on many threads at once (zgx-video's chunks, its tests) each made a device, which hung NVIDIA's driver. A lost device fails every renderer on it, and each makes a new one, as before.
