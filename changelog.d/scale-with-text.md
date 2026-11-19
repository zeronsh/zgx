---
section: added
---
- **An app's own text tokens can follow the system's text size** (zgx, zgx-script): `Themes::scale_with_text` and `ui.theme.scale_with_text { "app.prose.size", … }` have GNOME's text scaling and Windows' Text size multiply the tokens named, as they do `font.size.*` and `font.line.*`. Before, an app's own type scale (a chat's reading column) stayed put while the interface grew around it, and read smaller than its sidebar at Large Text.
