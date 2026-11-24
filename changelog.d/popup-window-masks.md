---
section: fixed
---
- **A masked view draws in a popup's own window** (zgx-gpu): a node with a mask (`mask`, `mask_gradient`, `mask_radial`) inside a popup shown in a window of its own, or on the overlay plane above a native view, drew nothing at all. Those renderers draw their items from the plane's corner, and the mask was placed from the window's, so it lay off its node by the popup's position and hid all of it. It is placed from the same corner now.
