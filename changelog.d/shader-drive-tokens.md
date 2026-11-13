---
section: fixed
---
- **A clocked shader's theme-token parameters draw the token's colour** (zgx, zgx-script): with `shader_clock = true`, a row of `shader_params` given as a theme token (`"$accent"`) drew as zeros, fully transparent, because the engine's drive wrote the literal row over it. The drive now resolves tokens where the node is (`ShaderDrive::tokens`) and follows the theme as it changes, without putting the clock back. `layer_params` takes theme tokens too, under `layer_clock` or without it.
