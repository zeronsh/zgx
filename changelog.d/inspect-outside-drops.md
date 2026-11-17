---
section: added
---
- **`zgx inspect` drives drags from outside the app, and reduced motion** (zgx-script): `files hover|drop|leave <target|x,y> <path…>` (files from a file manager: `on_file_hover`, then a `FileDrop` each and `FilesDrop`, `on_file_drop`), `text-drop <target|x,y> --text T [--html H] [--hover-only]` (text from another app, into a field or a `drop_when`), and `prefers reduced-motion on|off|system` (what this process reads as the system's preference, never its setting), through the same code the desktop host's events take; MCP tools `zgx_files`, `zgx_text_drop`, `zgx_prefers`.
- **`zgx inspect wait --text` is met by what a field holds** (zgx-script): an input's or an editor's value counts as well as the text a node says, so waiting for typed or dropped text into a labelled field no longer times out; a timeout names the value it held.
