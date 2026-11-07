---
section: fixed
---

- A window at rest gives back the vertex memory its largest frame took (a long list scrolled, a big page): on Vulkan, GL and the other plain upload paths its vertex buffer stayed at that frame's size and its staging chunks were never freed. On Apple silicon and Direct3D 12, the buffers the GPU is done with are dropped even when rest comes right after a frame.
