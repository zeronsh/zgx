---
section: added
---
- **A 3D world shades hundreds of point and spot lights** (zgx-3d, behind the new `lighting` feature): with more than eight, or with `World::set_clustered_lights(true)`, its view is cut into 16 by 9 tiles and 24 depth slices, each light listed in the clusters it reaches, and each pixel shades with its own cluster's lights alone. The lists are made on the CPU, again only when a light, the camera or the view's size changes. Without the feature, or with eight lights or fewer, a world draws as before, with the eight lights nearest the camera's target.
