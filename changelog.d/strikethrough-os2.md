---
section: fixed
---
- **A line-through is drawn where Blink draws it** (zgx, zgx-gpu): the font's ascent is rounded in device pixels, the stripe's top is `2/3` of it down from the line's top less half its thickness, the top is rounded to a device pixel and the height floored to whole device pixels (at least 1), as Chrome's `ComputeLineThroughLineData` and `DecorationLinePainter` do. A default decoration (`strike = true`, `Decoration::default()`) is a tenth of the font size thick; `Decoration::new(t)` keeps the thickness asked for, rounded to a device pixel. Geist at 13 pt at 2x fills device rows 10 and 9 above the baseline, as Chrome does. A face without an ascent keeps 0.3 em.
