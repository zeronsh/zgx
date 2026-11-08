---
section: fixed
---
- **A followed scroll view follows again only when moved to its end** (zgx): content shrinking up to a view left above its end (a fold closing under the reader) clamps it there, and `follow_end` no longer takes that clamp as asking to follow: what arrives next leaves it where it is. A move to the end (a wheel, a script, the scrollbar), or a wheel down at the end with nowhere further to go, follows again. Until a frame's layout settles, the clamp only shows: a shorter size a pass goes through on the way no longer moves the view.
