---
section: fixed
---
- **Look Up in a text area shown a line at a time**: `EditorHandle::look_up` asked for the baseline of a text the editor doesn't lay out (its lines are nodes of their own), and returned false. It now takes it from the line node holding the selection's start, as `text_area` takes its pieces.
