---
section: fixed
---
- **A context menu keeps the selection it opened on drawn**: right-clicking inside an editor's selection opened the menu, which took the focus, and the highlight went (only the window going to the back kept it). While a context menu opened on the editor (its own, or the app's on it or a view holding it) shows, the selection stays drawn in the colour of a selection in the back, as AppKit's does, and is drawn as before once the menu closes.
