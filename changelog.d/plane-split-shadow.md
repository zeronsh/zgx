---
section: fixed
---
- **A box's shadow reaching a native view no longer lifts the box over its own page** (zgx, zgx-desktop): a card with an outer shadow (a drop shadow reaches three blurs out) whose shadow touched another page was drawn on the overlay plane, fill included, over the page it held (a pane's web view invisible in a light theme, where the shadow shows, and fine in a dark one). The plane is now chosen by where the box and its outline paint; a shadow reaching a page goes under it.
