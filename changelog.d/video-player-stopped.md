---
section: fixed
---
- **A script hears its video player stop.** `ui.shared(name).playing` stayed true after `ui.invoke("<name>.play", false)` until another frame came, and a stopped player sends none: it's told as the player is played or stopped.
