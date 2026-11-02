---
section: changed
---
- **A picture that can't be shown is told, never a script error** (zgx-script): a `ui.image`, `ui.svg` or `bg_image` whose file is gone or isn't a picture draws nothing, shows the image's `fallback` (now for local files too, not only remote ones) and is told to its new `on_error(why)`, else warned of once, with the script line naming it. It was a script error, which in a gallery stopped every window's reloads, though a file can go at any time. `on_load({ w, h })` tells a picture's size once known (a file's from its header, before it draws; a remote one's as it arrives), and `ui.image_size(src)` reads it from the header alone, nothing decoded, so a design system can hold a picture's place at its final size.
