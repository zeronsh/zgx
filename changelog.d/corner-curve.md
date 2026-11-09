---
section: added
---
- **Boxes can have iOS's continuous corners** (zgx, zgx-gpu, zgx-script): `corner_curve` (`CornerCurve::Continuous`, Luau `corner_curve = "continuous"`) draws a box's fill, border, shadows and sharp inset shadows with `CALayer`'s `cornerCurve = continuous`: the function QuartzCore's own shader runs, within 2 levels of 255 of `CALayer`'s pixels at 2x and 3x on the boxes checked, simulator only. It is off unless asked for, and a window with none compiles and runs nothing new. For now such a window takes the wgpu path on macOS and iOS, Direct3D 11 draws the corners circular, and what the box clips, blurred inset shadows, a backdrop blur and a shader fill stay circular.
