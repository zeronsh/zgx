---
section: performance
---
- **Text shapes an ASCII word without looking up each character's script** (zgx-cosmic-text): an ASCII run's script is Latin if it has a letter and none otherwise, so it's read from its bytes rather than guessed one character at a time. A dashboard whose values change every frame flushes 0.78% faster (heavy-both, 1,120 → 1,111 K instructions a frame), drawing exactly as before.
