---
section: performance
---
- **Variable-height lists pay for keeping their place only when they load earlier pages** (zgx): the key of the row a list holds in view, kept so rows added above it leave the view where it was (`on_start`), is no longer carried by every list's scroll and layout pass.
