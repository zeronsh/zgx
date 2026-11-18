---
section: fixed
---
- **A popup with no compositor is cut to the panel it shows, however deep or inset** (zgx-desktop, X11): its window was shaped to rounded corners only when the popup's root, or a child its very size within four levels, was the rounded panel, so a menu whose panel is inset in padding or nested under wrappers (Zeron's) showed black corners and a black inset under a tiling window manager with no compositor. The panel is now the largest box in the popup that paints a fill, at any depth, and the window is cut to it, inset and corners.
