---
section: performance
---
- **A scroll under a floating frosted panel copies its pixels** (zgx-gpu): a backdrop-blurred panel over a scroll view that doesn't repaint (a composer over a transcript, its shadow reaching into the view) no longer makes every frame repaint the whole view and blur it in full. Its blur reads only within its own box, so it repaints where it is and where the copy moved it, blurring what moved under it afresh, and the rest of the view is copied. Layers over the view are carried the same way; only one reading what lies under it (a blend, a shader reading the backdrop) still repaints the view.
