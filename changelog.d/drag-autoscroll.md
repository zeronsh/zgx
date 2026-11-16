---
section: added
---
- **A drag held near a scroll view's edge scrolls it**, as a browser's does: the window's own drag (`on_drag`, `start_drag_from`), another window's or app's, and files from outside. The innermost scroll view under the pointer that can still move toward that edge scrolls while the pointer is within 48 points of it, up to 24 points a 16 ms frame (full speed from 12 in), on either axis, and what is under the pointer hears the drag over it again, so a drop indicator follows the content. Nothing is scheduled in the middle, at the end of what scrolls, or once the drag leaves, drops or ends. zgx-dom's in-app drags scroll alike; the browser scrolls for its own.
