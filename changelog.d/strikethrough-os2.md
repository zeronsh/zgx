---
section: fixed
---
- **A line-through sits where the font puts it** (zgx, zgx-gpu): its top is the face's OS/2 yStrikeoutPosition above the baseline and, for a decoration left at the default thickness (`Decoration::default`, `strike = true`), its thickness the face's yStrikeoutSize, as Chrome places it; a face without them keeps 0.3 em and the decoration's own thickness. Geist at 13 pt: top 4.13 above the baseline (was 3.90), 0.65 thick. A decoration made with `Decoration::new(thickness)` keeps the thickness asked for.
