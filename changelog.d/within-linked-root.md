---
section: fixed
---
- **A script's files are found under a linked directory** (zgx-script): a root reached through a symbolic link (macOS's temporary directory, a linked project folder) failed its containment check for every file that exists, so a `.gltf`'s buffers came up "not found". It's as strict as before: a link in the root that reaches out of it is still out.
