---
section: performance
---
- **A frame looks only at observed sizes that changed** (zgx): after a layout, every node whose size is observed (`on_resize`, a canvas's size, a terminal's rows) had its size worked out and gathered, then compared before publishing. Now each is compared as it's worked out, and only those resized are gathered. heavy-scroll −0.115% and layout-drag −0.105% instructions; a terminal at 64 rows −1.9%.
