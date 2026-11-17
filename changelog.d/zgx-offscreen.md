---
section: added
---
- **`ZGX_OFFSCREEN=1`** (zgx-desktop, macOS): test and probe sessions keep their windows off anyone's screen. No window is ever ordered in: made hidden, a show, focus, restore, fullscreen or zoom of one does nothing, popups stay inside their window, and no native menu, prompt, file panel, web alert, look-up, notification or browser shows; WindowServer lists none of its windows on any screen. Its windows are also invisible and click-through, out of Mission Control and window cycling, and the app takes no Dock icon or Cmd-Tab place and never activates. It goes on drawing as the app has its windows shown (it implies `ZGX_RENDER_WHEN_COVERED`) and answering `zgx inspect`, whose snapshots are zgx's own frames.
