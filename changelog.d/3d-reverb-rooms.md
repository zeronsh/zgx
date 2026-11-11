---
section: changed
---
- **Reverb zones each ring in a reverb of their own** (zgx-3d, `audio` feature). A world's zones play through two reverbs, each on a bus of its own under the world's: the listener's zone and the one it's fading from. Each emitter sends to them by how much the listener hears each zone, and half again more when it's inside the zone itself. A zone the listener has left rings out rather than being cut off. Its reverb goes to another zone once its decay has passed. An emitter starts with its sends, so its first sound reaches its room. The first zone takes two of the engine's buses, and the world keeps them for its life.
