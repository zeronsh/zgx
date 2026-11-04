---
section: fixed
---
- **Nothing takes the pointer inside `pointer_events = "none"` but what has handlers of its own** (zgx, zgx-script, zgx-desktop): a `ui.show`, `ui.switch` or `ui.presence` holding such a layer took every press over the layer's box, and in a script any node with an `id`, a class, a ref, `on_mount`, `on_resize` or `on_visible` took the pointer as if it had handlers. A contents node has no box to hit now, and an element listens only once a draggable asks it to. The cursor now comes from what a press would reach, past what the pointer passes through, unless that says a cursor of its own (`InputDispatcher::cursor_at`). `NodeHandle::on_event` listens from after mount.
