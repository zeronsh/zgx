---
section: fixed
---
- **A clear node over a scroll view no longer stops its scrolls from copying** (zgx-gpu): a box or rectangle that paints nothing (an overlay scrollbar's track away from the pointer, over every list it scrolls) counted as drawing over the content, so each scroll repainted the whole view where it could have copied its pixels. It now counts as what it paints: nothing.
