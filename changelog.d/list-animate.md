---
section: added
---
- **`ui.list` takes `animate`, as `ui.each` does** (internal, subject to change): rows the list places elsewhere (a row growing above them, a row coming in between, a drop settling) slide from where they showed instead of jumping, on the compositor. A row coming into view starts where it would have been, and one leaving it slides out before it goes. Scrolling moves none, a list without it costs nothing more, and reduced motion places them at once.
