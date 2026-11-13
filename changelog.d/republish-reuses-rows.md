---
section: performance
---
- **A republished shared state converts only the rows that changed** (zgx-script): after `Shared::set` reconciles a long list, a script's first read derives its table from the last one's, as after its own `set_key`, so a 1,200-row list with one edited row converts about 1 row instead of all (also for lists inside maps, down 4 levels).
