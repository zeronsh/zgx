---
section: added
---
- **Canvas text takes line height, letter spacing and a wrap width** (zgx-script, zgx): `c:text(text, x, y, { line_height, letter_spacing, width })` spaces lines and characters in points, as a text node does, and with `width` wraps to lines that wide in a box that wide, which `align` places on `x` and aligns its lines in. `c:measure_text` takes the same options, so a paragraph's height is `select(2, c:measure_text(body, { width = 240 }))`. In Rust, `CanvasText::width` and `canvas::measure_text_in`.
