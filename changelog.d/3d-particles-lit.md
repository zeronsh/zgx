---
section: added
---
- **3D particles can be lit by the scene** (zgx-3d, `particles` feature): `Emitter::lit(Lit { phase })` makes an emitter's colour an albedo lit by the sun, the nearest point and spot lights and the sky, each light scattered toward the camera by a Henyey–Greenstein phase, so rain glows toward a lamp behind it and is dark where nothing lights it. Unlit emitters are drawn as before, at the same cost.
