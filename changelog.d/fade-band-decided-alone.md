---
section: fixed
---
- **A fade's band repainted alone draws as a whole repaint draws it** (zgx-gpu): each band of a fade (`fade_edges`) is drawn as one, through a texture, only where what it holds stacks, and that is now decided band by band. It was decided for all of a fade's bands together, from those the frame repainted: a scroll view whose bottom band stacked (a code block's text on its box) drew its top band as one in a frame repainting both, and item by item in a frame repainting the top band alone, so a frame after a scroll could change a few pixels of the top band (a glyph's overlap with the next one, a box's shadow under it) that a whole repaint draws otherwise.
