---
section: fixed
---
- **A GPU view has a texture budget of its own** (`zgx-gpu`): a 3D scene's texture shared 64 MB with pictures, and past it the frame stopped with "visible image and GPU view textures exceed their budget", though pictures may rightly hold that much (a video's large wallpaper, or a clip's past frames). Views now answer to their own budget, which a view truly past still fails, saying what holds it. Pictures not showing still give their textures back first; nothing on screen goes softer or blank for a view.
