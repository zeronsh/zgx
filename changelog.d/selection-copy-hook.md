---
section: added
---
- **A selectable region says what a copy out of it puts on the clipboard** (zgx-script): `on_copy = function(request)` on a `selectable` node hears the selection's `text`, `html` and `leaves` (for each text node it spans, in order, the id of the element it is in inside the region, or `false`) and returns `{ text, html }`, or nil for the selection's own. ⌘C, the Edit menu and the selection's context menu all ask it; a region without one copies as before. For citations: a quote copied with its source.
