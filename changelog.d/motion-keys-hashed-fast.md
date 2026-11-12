---
section: performance
---
- **A list animating its rows hashes their keys faster** (zgx-script): `ui.each` with motion keeps its rows in maps hashed with zgx's seeded key hasher instead of std's SipHash, and sizes them for the list up front. The script-layout benchmark does 2.4% fewer instructions.
