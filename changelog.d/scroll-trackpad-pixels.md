---
section: performance
---
- **A trackpad's scroll moves in whole device pixels, and copies** (zgx-desktop, zgx-gpu): a trackpad's and its coast's motion came in fractions of a pixel (0.3 pt one refresh, 3.37 the next), and a scroll moved by a fraction repaints its whole view, so a trackpad got none of the copies a wheel gets. Its motion now goes out in whole device pixels, what's short of one carried to the next refresh, and a move counts as whole to 1/64 of a pixel, which f32 offsets far down a long list or at 1.5x need. Zeron's transcript scrolled by trackpad went from no frames copied to all of them.
