---
section: added
---
- **WebP images** (zgx-gpu, zgx-desktop, zgx-script): `ui.image` and the decoders read lossy and lossless WebP (`image`'s pure-Rust decoder; an animated file shows its first frame), kept as drawn like PNG and JPEG and shrunk to the size they are drawn at. A WebP decodes whole before it is shrunk (it has no scaled decode), so a big photo peaks at its full size for a moment. New `webp` feature, on by default, in zgx-gpu, zgx-desktop and zgx-script.
