---
section: added
---
- **A span cut into pieces can draw as the span whole: `ui.span { …, continues = true }`** (zgx, zgx-gpu, zgx-script): `TextSpan::continues()` (`TextRun::continues`) marks a span as the next piece of the span before it, one span cut into pieces (a streamed reply's fade, each piece a colour of its own). Marked pieces alike in font, size, padding, background and radius are padded at the span's ends alone, their washes and underlines drawn as one, so a chip cut by a fade wraps and draws as the same text written at once. Spans not marked draw as before, each its own: two chips side by side stay two chips, two links two underlines.
