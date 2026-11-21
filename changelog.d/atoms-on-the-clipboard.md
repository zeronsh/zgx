---
section: added
---
- **A text area's atoms go through the clipboard** (zgx, zgx-script, zgx-dom): a copy or cut of text with chips puts them beside the text as `application/x-zgx-atoms`, and a paste into a text area that takes atoms (a textarea with `atoms`) makes them again, with their `id`, `data` and `style`, in one undo step. Within the app they come back exactly as copied; from another window, app or page a textarea reads their names, data and looks back, giving one a new `id` where the target already has its name. A plain-text target, or another app, gets the text alone.
