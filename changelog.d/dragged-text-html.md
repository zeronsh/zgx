---
section: added
---
- **Text dragged in from another app carries its HTML too** (`DraggedText::html`, hidden while its API settles): where the source offers it (a browser's or a document's selection: `public.html` on macOS, `text/html` on X11 and Wayland, `CF_HTML`'s fragment on Windows), a destination taking `DraggedText` gets it beside the text, to drop rich content as it pastes HTML.
