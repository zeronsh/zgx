---
section: fixed
---
- **A bundle's `zgx.json` is checked as its other files are** (zgx-run): the window and host options of an app opened from a bundle (`zgx run <bundle>`) were read from the file on disk unchecked, so one changed since `zgx build` could change its window. Read through the bundle's checked files now, one changed or removed since it was built is refused.
