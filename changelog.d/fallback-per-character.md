---
section: fixed
---
- **A character a font lacks falls back alone, and to the system sans on macOS** (zgx-text): in text asked for in a family that isn't installed, a word holding a character such as `…` was drawn whole in the fallback face for that character (Times), apart from the words beside it; it now takes the same face as they do, and only characters that face lacks come from another. On macOS a character missing from a Latin-script font comes from the face Core Text substitutes from the system font (SF, at the text's weight), as AppKit draws it, not Chrome's standard font, Times: Geist without `…` drew it in a serif. Japanese, Korean and Chinese text keeps its standard fonts.
