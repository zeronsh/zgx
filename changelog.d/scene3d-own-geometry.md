---
section: added
---
- **`ui.mesh` takes its own triangles** (zgx-script): `geometry = { positions, indices, normals, colors }` draws a script's mesh (a CAD model, a scan, generated terrain), with `face_normals` and `face_colors` (or `flat = true`) for a flat-shaded, per-triangle look, and clicks hit its triangles as they do a named shape's. Inputs are checked with messages that say what's wrong, and a mesh holds at most 4,194,304 vertices as drawn. `material.double_sided` draws back faces too, for an open shell or a cut-away. Until now a script could only show the named shapes or a glTF file from its folder.
