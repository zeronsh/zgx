---
section: added
---
- **`WebViewOptions::accelerated`** (Luau `accelerated`; Linux only, macOS and Windows ignore it): on by default. Off, WebKitGTK paints the page on the CPU and zgx copies and uploads only what each frame changed, as up to four rectangles, instead of the whole page WebKit's compositor reports on every frame. A 40 × 40 spinner on a 1076 × 700 page uploads about 3,300 pixels a frame instead of 753,200, and WebKit's web process drops from 40–52% of a core to 3–9%. Painted pages draw no `backdrop-filter` and flatten 3D transforms.
