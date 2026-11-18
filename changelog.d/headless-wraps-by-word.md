---
section: fixed
---
- **Text laid out without a renderer breaks where the app's does** (zgx): the fallback layout (`FallbackTextLayout`, what `zgx test` and a scene with no shaper use) wrapped by grapheme, splitting any word that didn't fit at a line's end, a link span after a sentence too. It now wraps by word as the shaper does (UAX #14 over the whole text, so a break where one span ends and the next begins is a break like any other; spaces hang past the edge; a word wider than the line breaks by grapheme), and centred or right-aligned plain text is aligned row by row. A test lays a corpus out both ways and asserts the same line breaks.
