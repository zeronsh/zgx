---
section: changed
---
- **A list's, a scroll view's and a text input's viewport lays out within it** (zgx): each is absolute in its box now, and a width change lays that box out alone, so panes of them cost one pass, not one more for each. Nothing here changed size in the layouts compared with the last release, at any window size, but a list or scroll view with no size of its own gets its size from nothing it holds: give it a size (`h`, `h_full`, `grow`), which is what it needed before. A text input or area a percent of a parent its content sizes stays in its flow, and costs what it did.
