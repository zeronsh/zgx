---
section: added
---
- **Container queries** (zgx, zgx-script): `container = true` on a box and `queries = { { min_width = 900, style = { … } } }` on what's inside give styles that follow the box's content size, as CSS's `@container` rules, in the frame layout changes it and without running the script; a resize that crosses no bound restyles nothing. As in CSS, a container's content doesn't size its width (a debug build reports one that's 0 wide). In Rust, `View::container` and `View::when_container(Query::new().min_w(900.), …)`, and `Scene::set_contained`.
