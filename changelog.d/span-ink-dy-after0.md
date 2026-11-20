---
section: added
---
- **`underline_skip_ink = true` on a span** (zgx, zgx-gpu, zgx-script): its underline breaks around the parts of glyphs that cross it (the tails of g, j, p, q, y), padded by its thickness each side, as CSS's `text-decoration-skip-ink: auto` does; the glyphs' outlines decide, so it follows any font. `Decoration::skip_ink`.
- **`icon = { dy = … }` on a span's icon** (zgx, zgx-script): moves the icon down by that many points (negative: up) without a taller line, for web's inline icons at `vertical-align: -3px` (1 below centre). `SpanIcon::dy`.

### Fixed

- **A chain of `ui.after(0)` timers no longer runs for ever inside one turn** (zgx-script): a callback that starts `ui.after(0, step)` made `app:frames(n)` in `zgx test` (and a frozen or virtual clock) never return; a zero-delay timer started from inside a timer's callback now waits a millisecond, as a window's next-frame timer does, so the chain advances with time. A real window's timers were never zero.
