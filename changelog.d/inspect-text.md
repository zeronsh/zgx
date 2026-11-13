---
section: added
---
- **`zgx inspect text <target>`** (zgx, zgx-gpu, zgx-script): how a text node was shaped: the family each run asks for and what it resolves to in the font database, each line's baseline, ascent and descent (the strut included), and the face (family, PostScript name, weight class) each run of glyphs was drawn in. `Scene::describe_text`, `Scene::set_text_describer`, `LayoutRun::ascent`/`descent`; MCP tool `zgx_text`.
