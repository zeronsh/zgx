---
section: fixed
---
- **Canvas text is drawn in the drawing's order and cut by its clips** (zgx): a shape drawn after a text covers it, where text used to sit over every shape whatever the order (so where two cards overlapped, the lower one's words showed through the upper), and `c:clip` cuts text drawn while it is open, to the clip's box (a rounded rectangle's or a circle's corners too), where text used to ignore clips. A canvas whose later shapes keep clear of its text, or whose text comes last (a chart's labels, a terminal's rows), draws and costs as before: the text goes under a later shape only where that shape's bounds meet a text's, which starts another layer, up to 8. `Canvas::layers`, `Canvas::layer`, `Canvas::text_clips` and `Canvas::text_after` say how.
