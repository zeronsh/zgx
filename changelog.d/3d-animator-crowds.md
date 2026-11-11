---
section: performance
---
- **A crowd playing one clip in step samples it once a frame** (zgx-3d, feature `gltf`): animators each playing a clip alone at the same time (added and played together) share the first one's sampled pose, each copying only the parts of nodes the clip moves. 500 characters of 64 joints in step cost 9% less a frame than the same crowd each at its own time.
