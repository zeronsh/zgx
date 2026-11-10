---
section: performance
---
- **Direct3D 12 starts drawing quads without compiling their shaders:** the six draw stages ship as FXC bytecode and pass straight to the driver when the compiler and flags match. Other shader modules still compile on demand.
