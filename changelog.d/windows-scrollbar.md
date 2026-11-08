---
section: changed
---
- **Scrollbars on Windows behave as WinUI 3's** (zgx): an overlay scrollbar is a thin indicator that widens to 6 in a 12 track 0.4 s after the pointer arrives and narrows 0.5 s after it leaves, each over 167 ms on WinUI's curve, rounded 3, 1 from the edge, its thumb at least 30 long and one colour under the pointer (the text at half, over the surface's colour as it widens); it goes 2 s after scrolling stops, fading over 83 ms. Where scrollbars always show, it is wide. Each wait is a single wake: a window at rest schedules nothing. macOS and Linux keep zgx's scrollbar.
