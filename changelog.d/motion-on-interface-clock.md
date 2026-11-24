---
section: fixed
---
- **Drags and throws keep time on the interface's clock** (zgx, zgx-script): a drop target's spring-loading delay, a selection drag's autoscroll, an anime draggable's throw and a 3D view's flick read the wall clock, so on a clock of the interface's own (a test's, a video made offline) a slow or loaded machine changed when a target sprang open, how far a selection scrolled, how far a draggable was flung, and whether a flick coasted. They now read `zgx::clock::now()`, the wall clock in a window, as before. Chord timeouts, find's counting and type-to-select keep the wall clock: they measure a person's real pauses.
