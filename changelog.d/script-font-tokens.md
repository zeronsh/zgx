---
section: fixed
---
- **A font token a script names a family for takes it**: a script's theme override (`theme = { ["font.sans"] = "Inter" }`) or `ui.theme.tokens` gave a font token a family's name as text, which the theme refused for a font token, so the override was dropped and text kept the default family. It is read as a theme file reads a font token's word: a CSS font-family list, its first family. Thanks to @katulevskiy.
