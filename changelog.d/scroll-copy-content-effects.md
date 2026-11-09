---
section: performance
---
- **Scrolling copies pixels past effects that move with the content**: tables and code blocks fading at their edges, clips fading at their foot and layers inside a scroll view no longer make it repaint its view each frame. A fade the view's edge cuts repaints just what it holds; a backdrop blur, a blend, a shader reading the window or the backdrop, or a filtered layer under a fade still repaints.
