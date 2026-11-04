---
section: fixed
---
- **A `ui.list` whose `key` fails keeps its rows** (zgx-script): the rows it knew keep the keys they had, with their nodes and states, and the error is still reported. Every row used to take its index as its key, so a slip remade the whole list.
