---
section: fixed
---
- **Animations run in the browser** (zgx-dom): a DOM view now has a frame clock, so `ui.tween`, `ui.spring`, `ui.spring_to`, `anime.animate` and a Rust view's `cx.frames()` tick there, where they never moved (and `cx.frames()` panicked) before. The page's `requestAnimationFrame` drives it, asked for only while an animation waits for a frame: a page at rest asks for none, and a hidden tab's are held back by the browser.
