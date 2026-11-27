---
section: added
---
- **A text area tells the width its text wraps in, and a line aligns in its own** (zgx): `EditorHandle::wrap_width` is the width its text wraps in, and `on_wrap_width` is told as it changes, after layout and before the frame is drawn, so line insets set there (a picture's height from the width) are in that frame. `LineInset::align` centres a line or places it right in the width it wraps in, its carets, clicks, selection and paint together (a picture's caption); `group_width` is how wide a scroll group shows. `LineInset` has a builder for each field, named as the field is (`hidden(true)`, `scroll_group(3)`, `nowrap(true)`; all hidden while they settle). A text area without them lays out and draws as before.
