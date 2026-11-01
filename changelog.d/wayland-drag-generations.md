---
section: fixed
---
- **A drag let go as an earlier drop finishes drops on Wayland** (zgx-desktop): once a drop is performed, the window's own drag ends there, taken, and its next drag goes to the compositor at once, while the earlier drop's target is still finishing with it; that drop's end, coming later, is no longer taken for the newer drag's. A tab grabbed straight after a drop moves to another window or tears off as one grabbed later does ([#1259](https://github.com/zeronsh/zgx/issues/1259)).
