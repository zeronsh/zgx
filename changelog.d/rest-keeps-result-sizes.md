---
section: performance
---
- **A window at rest keeps the sizes of a node's other layout results, not their children's places** (zgx): about 11% less memory per node on a list of rows. The first layout after rest that needs one lays it out again.
