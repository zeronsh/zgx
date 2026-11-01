---
section: performance
---
- **Script state finds and compares table keys without calling libc** (zgx-script): a table of a few fields is scanned for an equal key, and keys and short strings compare with a few word loads. A republished list matched by `id` costs 0.7% less an update (script-update), and its cost no longer moves with where memcmp's bytes land from build to build (#1273).
