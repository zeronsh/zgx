---
section: fixed
---
- **Colour emoji keep their colours** (zgx-gpu): a colour glyph (an emoji) was multiplied by its text's colour, so grey text dimmed its emoji, black text drew silhouettes and red text tinted them red. Colour glyphs now draw in their own colours with the text's alpha (opacity and fades still apply), whatever the text's colour or selected-text colour, and their shadows are their shape in the shadow's colour. The software renderer already drew them so.
