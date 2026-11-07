---
section: fixed
---
- **Scrolling goes to the view under the pointer again, not the one last scrolled** (zgx): a scroll gesture's latch held for as long as scrolls kept coming within 300 ms of each other, a trackpad's coast included, so a new swipe over another view (or a click on it, then a swipe) still moved the last one. As AppKit's, a latch now holds for its gesture and that gesture's momentum only: a trackpad's next gesture, or its momentum's end, lets go, and a press always lets go.
