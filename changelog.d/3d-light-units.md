---
section: added
---
- **3D point and spot lights in physical units** (zgx-3d, `lighting` feature): `Light::point(position)` and `Light::spot(position, direction)` take their brightness in lumens or candela, a `range`, a spot's `cone` and a `radius` that softens a light near what it lights and widens its highlights; `World::add_point_light` adds one. `Light::Point`'s `intensity` was and stays candela.
