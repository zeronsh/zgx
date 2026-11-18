---
section: fixed
---
- **An app starts on an X server without RandR** (zgx-desktop, through winit): winit asked the server for RandR's version and panicked without the extension (`Xvfb -extension RANDR`, some VNC and bare servers). Without it the root window is now the one monitor, at its size and scale, and screen changes go unheard.
