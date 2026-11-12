---
section: added
---
- **`ui.decode_image(base64)`** (zgx-script): a script makes a `Bitmap` from a PNG, JPEG or GIF's bytes, given in base64 (a picture a server or the host handed over, kept in a database), where it could make one only from the clipboard or alpha. It decodes off the main thread as `ui.image` decodes a file, takes up to 64 MiB, and rejects text that isn't base64 or bytes that aren't a picture. `ui.image { src = bitmap }` shows it.
