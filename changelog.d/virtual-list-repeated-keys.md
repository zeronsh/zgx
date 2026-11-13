---
section: fixed
---
- **A key given to two rows of a virtual list no longer panics** (zgx): the check asserted, the panic was contained once, and the pass after it took the app down. The first row now keeps the key and the others are left out, the same each pass, and the list says so once on stderr. Both the fixed-height and the measured lists.
