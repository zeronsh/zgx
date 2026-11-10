---
section: fixed
---
- **`zgx test` interrupted on Windows leaves nothing in the temp directory** (zgx-script): Ctrl-C or Ctrl-Break mid-run removes the directories its tests made before it ends, with a console interrupt's exit code (`STATUS_CONTROL_C_EXIT`), as Ctrl-C and SIGTERM already did on Unix.
