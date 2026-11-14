---
section: added
---
- **An app can read the clipboard's HTML** (zgx, zgx-desktop): `Platform::clipboard_html` returns what a browser or word processor copied, with its markup. On macOS that is `public.html`, on Windows the fragment `HTML Format`'s header marks, and on X11 and Wayland `text/html`. `clipboard_text` still reads the plain-text fallback. Writing was already there (`set_clipboard_rich_text`, HTML with its text), and Wayland now reads what it already offered. Hidden until its shape is settled.
