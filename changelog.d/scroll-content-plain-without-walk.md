---
section: performance
---
- **Scrolling content with nothing that paints as a layer skips its paint walk** (zgx): each scrolled frame used to walk every visible node of the content to confirm it could be copied. Now the scene keeps a list of the nodes that may paint otherwise (layers, edge fades, images with an effect). The list is brought up to date from what each frame repainted, and the walk is skipped when none of those nodes is inside the content. A transcript under a frosted composer scrolls with about 6% fewer instructions a frame; no setter does more than before.
