---
section: fixed
---
- **A layer over the window doesn't dim it** (zgx): a portal made a layer (`View::layer`: `ui.toast`'s host, a heads-up) or shown at a caret got a modal's backdrop (`backdrop`, `#00000082`) over the whole window, though a layer has none: every toast dimmed the window while it showed. Only a modal has one now.
