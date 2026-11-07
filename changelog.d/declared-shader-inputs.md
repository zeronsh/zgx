---
section: fixed
---
- **A script's layer shader with passes draws through them** (zgx-script): `ui.layer_shader(wgsl, { passes, images, geometry })` declares its shader for every renderer to start before it draws, and that start compiled it as a plain layer shader, which its inputs can't be. It failed to compile, and its layer showed as painted every frame. A declared shader with inputs now compiles with their bindings, as one first met in a frame does.
