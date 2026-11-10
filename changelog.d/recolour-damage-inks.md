---
section: performance
---
- **Rich text whose runs change colour alone damages those runs' glyphs** (zgx, zgx-gpu): a fade's frame, recolouring a streamed paragraph's newest words, damaged every line of the paragraph (its whole text's ink); it now damages, line by line, the boxes of the recoloured runs' glyphs, from the lines the renderer draws (`Scene::set_rich_text_inks`, installed with the renderer's text services), padded as a text's ink is for accents, descenders and underlines. Any other change to the text in the same frame, a text laid out otherwise (clamped, padded, right to left), shadowed, filled with a gradient or blurred, or a host without the renderer's text, damages its whole text as before.
