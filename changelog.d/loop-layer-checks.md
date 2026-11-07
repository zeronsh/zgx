---
section: fixed
---
- **A looping layer can't show where its node doesn't** (zgx-gpu, macOS): its place is set at every flush, frame or none, read back rather than trusted, and told again when it disagrees, which the log reports. A frame that fails hides every looping layer until one draws.
