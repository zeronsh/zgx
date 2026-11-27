---
section: added
---
- **A text area's line marks take corners, side borders, pictures and a turn** (zgx): `LineMark::decoration` gives a mark per-corner radii and per-side borders (a table's outer cells), `image` (`MarkImage::cover`) a picture filling it, cropped to its rounded corners (a note's image in its line's room above), `svg` an icon in its box, `align` a place across its line (left, centre or right), and `rotate` turns it about its centre (a toggle's chevron): a new angle eases there over `motion.duration.fast`, at once under reduced motion, and a settled mark schedules nothing. `LineMark`, and a scroll group's `GroupFrame`, have a builder for each field, named as the field is. A mark without them draws as before.
