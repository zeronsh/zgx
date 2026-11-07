---
section: added
---
- **Files open and save on iOS** (iOS). `ui.open_files` shows the Files picker (a file is copied into the app, a folder opened under its security scope, images alone go to the photo picker), and a file from `ui.save_file` goes to the export picker once `ui.write_file` has written it whole. `p:cancel()` dismisses a picker the script asked for. `Platform::file_written` tells a host a script's write finished.
