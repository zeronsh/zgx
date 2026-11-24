---
section: added
---
- **Text dragged into a page from another app drops on views that take it** (`zgx-dom`): a view with `on_drop::<DraggedText>` hears the browser's drag and drop as it does an in-app drag, its text and, where the source offered `text/html`, its HTML (`DraggedText::html`), as on the desktop. Elsewhere the browser's own drop is left alone.
