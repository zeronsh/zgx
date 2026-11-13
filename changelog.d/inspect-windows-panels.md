---
section: added
---
- **`zgx inspect windows` lists popup panels with their owner and state** (zgx-script): each window's line is followed by every popup it wants open (`popup <node>  menu  shown  owner main`), held back ones too (the app at the back), and its record carries `panels: [{ what, role, owner, shown, held, x, y, width, height }]` beside the existing `popups` list.
