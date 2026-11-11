---
section: fixed
---
- **A list's estimate no longer keeps every folded row open** (zgx-script): `ui.list { estimate = … }` read each row's fields to estimate it, and a folded row (`Value::folded`) read that way stayed opened for good, so a long transcript held all its rows unfolded. Each folded row now opens for its scan and closes again, and its scan is kept, so a settled row is never opened again for an added row or a new width. 15,000 folded rows under an estimate hold 24.5 MB rather than 37.3 at rest; an append costs 44M instructions rather than 115M, a new width 7M rather than 76M.
