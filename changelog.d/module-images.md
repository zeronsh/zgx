---
section: added
---
- **A native module shows images by name** (zgx-module, zgx-script): a module's thread writes a picture into an `ImageBuffer` (RGBA8, rows tightly packed) and a job shows it as `host:<module>.<name>` with `host.image`, its bytes allocated as the image's own so nothing is copied; `host.remove_image` removes it. What a module sets goes with its host, and as the module is loaded into its host again. [The guide](docs/modules.md) says how.
