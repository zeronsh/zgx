---
section: fixed
---
- **`zgx_sample` reads a layer shader's own pixels exactly, however far away** (zgx-gpu): the texture coordinates it read at came from the screen derivatives of the composite quad's, a little off, and that error times a read hundreds of points away blended in the texel beside the one meant, or landed in it: a shader reading data from its own pixels (a map, block ids) showed wrong cells, tearing and flicker, worse under zoom. The coordinates now come with the shader's uniforms, worked out from the quad.
