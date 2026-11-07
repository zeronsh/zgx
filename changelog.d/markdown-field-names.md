---
section: performance
---
- **A markdown document's blocks and runs name their fields without allocating** (zgx-script): each block and run made each of its field names anew, an allocation a name; they are made once a thread now. A 40 KB document shown costs 2.8% fewer instructions.
