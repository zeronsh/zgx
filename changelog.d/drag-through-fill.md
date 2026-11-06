---
section: fixed
---
- **A row that's all button can be dragged again** (zgx): since nodes with an `id` or `ref` stopped taking the pointer, a press landed on a clickable covering its draggable, which kept the press, and `on_drag` never started. A clickable covering most of its draggable now leaves the drag to it. A small control inside one, such as a tab's close button, keeps its press.
