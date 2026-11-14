---
section: performance
---
- **A scroll's frame gathers moved observed bounds with one allocation** (zgx): the list a frame's preparation gathers moved observed bounds into (an app's `observe_bounds`, and zgx's own: virtual lists, popovers) grew from nothing as rows moved, some seven allocations each scrolled frame in a transcript under glass; it now takes room for every observed node at the first that moved, once, and nothing is kept between frames. A frame where nothing moved allocates nothing.
