---
section: fixed
---
- **A list scrolled with the view around it follows a box clipping it** (zgx): a `virtualize = "ancestor"` list inside a box that eases open (or shut) now builds the rows the box shows as it grows, not only once the scroll view moves; before, reopened, it showed none until a scroll.
