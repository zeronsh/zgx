---
section: performance
---
- **A frame lets go of what the renderer keeps for nodes from those that changed, not a walk of all it keeps** (zgx, zgx-gpu): each frame whose content changed (a terminal's rows drawn again, a list scrolled) looked over every cached shape, raster and texture, and walked the whole scene for the images it shows. The scene now names the nodes removed (`Scene::resource_changes`); the renderer counts the images nodes show as it draws them and looks again only at those, so a frame with nothing removed and no image replaced prunes nothing.
