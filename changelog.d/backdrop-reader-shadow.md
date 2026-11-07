---
section: fixed
---
- **A layer shader reading what's behind it keeps what lies under its node's shadow**: a node with a box shadow and a shader calling `zgx_backdrop` composited over its whole layer, shadow included, but only what the shader could read was copied for it, so the window under the rest of the shadow came out clear.
