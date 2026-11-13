---
section: changed
---
- **The system font between Medium and Semibold draws emboldened, as Chrome's font synthesis does** (zgx-gpu, macOS): SF at weights 501 to 599 under 17.5 points gets Skia's fake-bold stroke (`size * lerp(size, 9..36, 1/24..1/32)`) on top of the interpolated outlines, widths unchanged. Measured in Chrome 157's Playwright channel at 12 to 17 px, `-webkit-font-smoothing: antialiased`: ink per column within 1%, +27 to +32% over the plain outlines; 500, 600 and over, and 17.5 px and over, are not emboldened. `tests/chrome_text_system.rs` checks the ratios.
