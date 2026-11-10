---
section: performance
---
- **A terminal row's same-style cells are built as a run** (zgx-terminal): the cells after one in its colours and flags, plain ASCII, are taken as it was rather than each looked over again (its colours resolved, its flags, cursor and selection asked), so building a row of text costs about half what it did.
