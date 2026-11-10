---
section: performance
---
- **A change inside an absolute box lays out that box alone** (zgx): a scrollbar's thumb moved in its rail, a terminal row's text runs, a badge, a menu's item no longer lay out every ancestor to the window's root and their other children each frame. The walk up from a change stops at the parent of the first absolute node it passes (placed by that parent, not by a `relative` block above), which is laid out again alone and stands if it answers its parent as it did; otherwise layout runs as before. A thumb in a fixed rail went from 36 measures a frame to 6, and a terminal row's labels from a count that grew with the screen's height to a flat one.
