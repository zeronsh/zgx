---
section: fixed
---
- **Plain glass blurs what is behind the window on Windows** (zgx-desktop): `Glass::plain` was Mica, which tints with the wallpaper and blurs nothing, so a window over other windows looked opaque. It is now the acrylic blur of what is behind the window, untinted, as gpui's `WindowBackgroundAppearance::Blurred` is, with no system backdrop and no frame extended under the client (DWM drew the frame over the blur). Windows 10 1809 and later. Glass of every other material keeps its Acrylic, Mica or Mica Alt.
