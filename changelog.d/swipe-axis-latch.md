---
section: fixed
---
- **A sideways swipe over a `scroll_x` in a page no longer stops it** (zgx): the swipe's slightest drift downward moved the page and latched the gesture there, so the view stopped until a pause. A horizontal view now takes the whole of a swipe mostly sideways, and a gesture stays with the view it latched to through a finger's drift, until a pause or a clear turn to the other axis.
