---
section: performance
---
- **A `ui.presence` with sizing of its own lays out without taffy** (zgx-script): its box is an overlay rather than a one-cell grid, laid out the same way on the native path, so presences nested around each paragraph of a chat no longer each run a grid layout. A presence that sets how its content is laid out (`items`, `justify`, `gap`, `grid`...) keeps the grid, which reads them as before. Content aligning itself (`self`) still aligns down, as in the grid, though an overlay aligns a child's `self` across: the box reads it down (`Scene::set_aligns_down`).
