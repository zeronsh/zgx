---
section: added
---
- **`ui.measure_text(text, style?)` gives the points a text takes, with nothing mounted** (zgx-script, zgx-dom): `{ w, h }` in a canvas text's font options (`size`, `font`, `weight`, `italic`, `tabular`, `line_height`, `letter_spacing`), wrapped at `max_w` if given. A table's column fits its widest cell this way. On the desktop it measures in the app's fonts, as `ui.render_canvas` does; in the browser, as the page lays the text out.
