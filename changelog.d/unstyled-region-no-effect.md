---
section: performance
---
- **A `ui.switch` or `ui.show` costs less to mount** (zgx): the region it shows its branch in, laid out as that branch and styled by nothing, no longer keeps a style effect and a subscription to the typography around it; what it shows inherits that typography directly. A library table scrolled a row at a time mounts about 2% fewer instructions.
