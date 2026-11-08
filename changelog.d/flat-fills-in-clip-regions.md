---
section: performance
---
- **A flat fill reaching into a rounded clip's corners draws its middle with the basic shader** (zgx-gpu): a page's background in its rounded card, or any `Rect` or panel interior crossing where its clip region changes pixels, ran the full shader and the region's lookup over every pixel. It now goes in pieces: the part where the region changes nothing, on whole device pixels, drawn without it (opaque, unblended when it is), and the bands around it in the region, with the same bytes. `gpu_frame_bench`'s new `ZGX_MODE=card` (a settings page's card repainting): GPU time a frame BEFORE → AFTER.
