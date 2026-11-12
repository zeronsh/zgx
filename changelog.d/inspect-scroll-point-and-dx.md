---
section: changed
---
- **`zgx inspect scroll` takes any target `click` takes, or a point, and a sideways motion** (zgx-script): `scroll <target|x,y> <dy> [dx]` sends one wheel event at a node's middle or a window point, with `dx` as a trackpad's sideways motion (the MCP tool `zgx_scroll` takes `dx` too). It no longer needs a reachable node: a target that doesn't show fails as `click` and `hover` do.
