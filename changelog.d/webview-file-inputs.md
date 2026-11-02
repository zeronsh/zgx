---
section: fixed
---
- **A web page's file input opens the window's file dialog on macOS and Linux** (zgx-desktop): `<input type=file>` asks through `WebView::choose_files`, the window's own file dialogs (a sheet on macOS; the desktop portal's, or zenity's, on Linux), and what's chosen goes to the page, several files where the input takes several and a folder for `webkitdirectory` on macOS. On macOS it opened nothing; on Linux GTK's chooser opened as a window of its own, apart from the app's.
