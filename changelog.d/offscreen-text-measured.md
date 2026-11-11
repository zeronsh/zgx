---
section: fixed
---
- **Text drawn by a renderer is measured by its text, even where the host installed none** (zgx, zgx-gpu): a scene mounted and drawn offscreen without `GpuRenderer::install_text` was laid out with zgx's estimate while the renderer drew real glyphs, so a word sized by its own width (a checkbox's or radio's label, a text beside a grown slider) was laid out narrower than it draws and lost its last glyph ("Lamps" drew "Lamp"). A renderer now makes its context's text services its thread's default (`Scene::set_default_text_services`), and a scene with none installed takes them at its next layout, right from its first frame. A host's own installing still wins.
