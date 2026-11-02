---
section: performance
---
- **A wrapping text keeps its lines while a window is dragged a few points** (zgx, zgx-gpu): a resize measured every wrapping text again at the new width, breaking its lines anew, though most rows had no break move. Text measured from its words now also reports the widths it holds for. From its widest line's width to where the next word would join a line, its lines break the same, and layout serves any width between from the size it has, without asking. A host's measurer gives them with `Scene::set_font_text_reach_measurer`, which `TextServices` and the renderer install. On a 3,000-row file list dragged 3 points a frame (`reflow_bench`), layout costs 33.0 M instructions a frame instead of 69.0 M, and text 0.13 ms instead of 1.90 ms.
