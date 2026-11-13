---
section: added
---
- **`ZGX_OFFSCREEN=1`** (zgx-desktop, macOS): test and probe sessions keep their windows off anyone's screen. Every window, popups included, is invisible and click-through, out of Mission Control and window cycling, and never activates; the app takes no Dock icon or Cmd-Tab place. It goes on drawing (it implies `ZGX_RENDER_WHEN_COVERED`) and answering `zgx inspect`, whose snapshots are zgx's own frames.
