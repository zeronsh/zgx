---
section: performance
---
- **A PNG drawn small is read row by row and averaged as it comes** (zgx-gpu): a kept PNG used to decode whole (48 MB for a 4000 by 3000 photo) before it was shrunk to the size drawn, and the allocator kept that peak; it now decodes at the largest whole fraction that still covers the drawn size (about 896 across for 448 points at 2x), holding a few MB at once. Drawn larger, it decodes again at the larger size; interlaced PNGs still decode whole. JPEGs already decoded scaled.
