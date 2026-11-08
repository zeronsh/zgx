---
section: fixed
---
- **Documents opened from Finder or Open With reach the app on macOS** (zgx-desktop): an open-documents Apple Event's files were read with `stringValue`, which gives an HFS path ("Macintosh HD:Users:…") rather than a URL, so validation dropped every one. They are read with `fileURLValue` and arrive as `file://` URLs.
