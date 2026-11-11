---
section: performance
---
- **A fixed-size absolute box moved by its inset alone measures nothing** (zgx): a scrollbar's thumb whose `top` follows a scroll offset is placed where its parent's layout would put it, and arranged alone, with no layout of its rail or anything else. The move is read from its parent's last layout (the style that layout mapped for it) and taken only if those insets give where it is now; otherwise its walk is as before.
