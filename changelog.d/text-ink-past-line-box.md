---
section: fixed
---
- **Text ink may pass its line box** (zgx-gpu): a font taller than its line height (Segoe UI at 24/28) no longer loses its descenders; the clip goes a little past the box above and below, still inside the ancestors' clip, as CSS draws it.
