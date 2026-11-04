---
section: fixed
---
- **A popup's glass blurs on Windows** (zgx-gpu, zgx-desktop): a menu, popover or tooltip with `backdrop_blur` showed what's behind it sharp, as a popup's window has nothing of the app's to blur. It now blurs what its window last drew under it, taken again when the window repaints there, at the cost of a texture the popup's size while it shows and a copy per redraw. Past its window's edge, or over another popup, what's behind still shows through its fill.
