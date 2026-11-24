---
section: fixed
---
- **A canvas whose lines changed and then moved no longer fails with "stale scene node"** (zgx): the box holding a canvas's lines is laid out again as they come and go. It was marked and then missed, so what next moved the canvas (a sidebar closing beside a terminal printing) laid out lines already gone, and left the window half laid out.
