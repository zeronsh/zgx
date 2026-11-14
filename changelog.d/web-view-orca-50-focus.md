---
section: fixed
---
- **A Linux web view no longer makes Orca 50 lose the app's window** (zgx-desktop, `webkitgtk`): clicking or tabbing into a page focused GTK's offscreen window that draws it, and Orca 50, finding no top-level for that window, cleared its focus and left the app. GTK's side of a page (its window and view) no longer sends screen readers events or says it is active or focused; the page's own tree, which the app's window embeds, still reaches them.
