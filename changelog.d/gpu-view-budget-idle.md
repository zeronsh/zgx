---
section: fixed
---
- **A GPU view finds room among pictures not drawn this frame** (`zgx-gpu`): a 3D scene's texture over the 64 MB shared with pictures stopped the frame with "visible image and GPU view textures exceed their budget", even with the budget held by pictures nothing draws any more (a video clip's past frames). Pictures not drawn this frame now give their textures back first, least recently drawn first, as they do for a picture.
